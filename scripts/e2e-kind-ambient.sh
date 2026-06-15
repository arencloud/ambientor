#!/usr/bin/env bash
# Phase 3.5 e2e: kind + Istio ambient + bookinfo → assessment → plan → rollout → verify.
set -euo pipefail
cd "$(dirname "$0")/.."

CLUSTER="${AMBIENTOR_KIND_CLUSTER:-ambientor-e2e}"
CTX="kind-${CLUSTER}"
NS_SYSTEM="${AMBIENTOR_SYSTEM_NS:-ambientor-system}"
BOOKINFO_NS="${BOOKINFO_NS:-bookinfo}"
INVENTORY_NAME="${MESH_INVENTORY_NAME:-bookinfo-scan}"
ASSESSMENT="${INVENTORY_NAME}-assessment"
PLAN="${ASSESSMENT}-plan"
ROLLOUT="${PLAN}-rollout"
ISTIO_VERSION="${ISTIO_VERSION:-1.24.2}"
GATEWAY_API_VERSION="${GATEWAY_API_VERSION:-v1.2.0}"
E2E_TIMEOUT_SEC="${E2E_TIMEOUT_SEC:-1200}"
POD_READY_TIMEOUT_SEC="${POD_READY_TIMEOUT_SEC:-300}"
AMBIENTOR_OPERATOR_DEPLOY="${AMBIENTOR_OPERATOR_DEPLOY:-ambientor-ambientor-operator}"
AMBIENTOR_API_DEPLOY="${AMBIENTOR_API_DEPLOY:-ambientor-ambientor-api}"
POLL_INTERVAL_SEC="${POLL_INTERVAL_SEC:-5}"
SKIP_CLUSTER_CREATE="${SKIP_CLUSTER_CREATE:-0}"
SKIP_IMAGE_BUILD="${SKIP_IMAGE_BUILD:-0}"
SKIP_ROLLBACK_E2E="${SKIP_ROLLBACK_E2E:-0}"

# Waiting reasons that will not self-heal; fail immediately instead of kubectl wait's long timeout.
FATAL_POD_WAIT_REASONS=(
  ErrImageNeverPull
  ImagePullBackOff
  ErrImagePull
  InvalidImageName
)

log() { echo "[e2e] $(date -u +%H:%M:%S) $*"; }
die() { log "ERROR: $*"; exit 1; }

kubectl_ctx() { kubectl --context "${CTX}" "$@"; }

wait_for() {
  local desc="$1"
  shift
  log "wait: ${desc}"
  kubectl_ctx wait "$@" --timeout="${E2E_TIMEOUT_SEC}s"
}

wait_for_pod_ready() {
  local desc="$1" ns="$2" selector="$3"
  local timeout="${4:-${POD_READY_TIMEOUT_SEC}}"
  local start now ready total reason
  start="$(date +%s)"
  log "wait: ${desc} (up to ${timeout}s, fail-fast on image errors)"

  while true; do
    now="$(date +%s)"
    if (( now - start >= timeout )); then
      [[ "${ns}" == "${NS_SYSTEM}" ]] && dump_ambientor_diagnostics 2>/dev/null || true
      kubectl_ctx get pods -n "${ns}" -l "${selector}" -o wide || true
      die "timeout waiting for ${desc} after ${timeout}s"
    fi

    while IFS= read -r reason; do
      [[ -z "${reason}" ]] && continue
      for fatal in "${FATAL_POD_WAIT_REASONS[@]}"; do
        if [[ "${reason}" == "${fatal}" ]]; then
          [[ "${ns}" == "${NS_SYSTEM}" ]] && dump_ambientor_diagnostics 2>/dev/null || true
          kubectl_ctx describe pods -n "${ns}" -l "${selector}" || true
          die "${desc}: unrecoverable pod state (${reason})"
        fi
      done
    done < <(kubectl_ctx get pods -n "${ns}" -l "${selector}" -o jsonpath='{range .items[*]}{range .status.initContainerStatuses[*]}{.state.waiting.reason}{"\n"}{end}{range .status.containerStatuses[*]}{.state.waiting.reason}{"\n"}{end}{end}' 2>/dev/null | sed '/^$/d')

    ready="$(kubectl_ctx get pods -n "${ns}" -l "${selector}" -o jsonpath='{range .items[*]}{.status.conditions[?(@.type=="Ready")].status}{"\n"}{end}' 2>/dev/null | grep -c '^True$' || true)"
    total="$(kubectl_ctx get pods -n "${ns}" -l "${selector}" --no-headers 2>/dev/null | wc -l | tr -d ' ')"
    if [[ "${total}" -gt 0 && "${ready}" -eq "${total}" ]]; then
      log "ready: ${desc}"
      return 0
    fi

    sleep "${POLL_INTERVAL_SEC}"
  done
}

check_fatal_pod_states() {
  local ns="$1" selector="$2" desc="$3"
  local reason
  while IFS= read -r reason; do
    [[ -z "${reason}" ]] && continue
    for fatal in "${FATAL_POD_WAIT_REASONS[@]}"; do
      if [[ "${reason}" == "${fatal}" ]]; then
        [[ "${ns}" == "${NS_SYSTEM}" ]] && dump_ambientor_diagnostics 2>/dev/null || true
        kubectl_ctx describe pods -n "${ns}" -l "${selector}" || true
        die "${desc}: unrecoverable pod state (${reason})"
      fi
    done
  done < <(kubectl_ctx get pods -n "${ns}" -l "${selector}" -o jsonpath='{range .items[*]}{range .status.initContainerStatuses[*]}{.state.waiting.reason}{"\n"}{end}{range .status.containerStatuses[*]}{.state.waiting.reason}{"\n"}{end}{end}' 2>/dev/null | sed '/^$/d')
}

describe_pending_scheduler() {
  local ns="$1" selector="$2"
  local pod
  while IFS= read -r pod; do
    [[ -z "${pod}" ]] && continue
    log "scheduler events for ${pod}:"
    kubectl_ctx describe -n "${ns}" "${pod}" 2>/dev/null | sed -n '/Events:/,$p' | head -20 || true
  done < <(kubectl_ctx get pods -n "${ns}" -l "${selector}" --field-selector=status.phase=Pending \
    -o jsonpath='{range .items[*]}{.metadata.name}{"\n"}{end}' 2>/dev/null)
}

wait_for_deployment() {
  local desc="$1" ns="$2" deploy="$3"
  local timeout="${4:-${POD_READY_TIMEOUT_SEC}}"
  local start now selector app
  start="$(date +%s)"
  app="$(kubectl_ctx get deployment -n "${ns}" "${deploy}" -o jsonpath='{.spec.selector.matchLabels.app}' 2>/dev/null || true)"
  selector="app=${app}"
  [[ -n "${app}" ]] || die "deployment ${deploy} not found in ${ns}"
  log "wait: ${desc} (deployment/${deploy}, up to ${timeout}s)"

  while true; do
    now="$(date +%s)"
    if (( now - start >= timeout )); then
      [[ "${ns}" == "${NS_SYSTEM}" ]] && dump_ambientor_diagnostics 2>/dev/null || true
      kubectl_ctx get pods -n "${ns}" -l "${selector}" -o wide || true
      describe_pending_scheduler "${ns}" "${selector}"
      die "timeout waiting for ${desc} after ${timeout}s"
    fi

    check_fatal_pod_states "${ns}" "${selector}" "${desc}"

    if kubectl_ctx rollout status "deployment/${deploy}" -n "${ns}" --timeout=10s >/dev/null 2>&1; then
      log "ready: ${desc}"
      return 0
    fi

    sleep "${POLL_INTERVAL_SEC}"
  done
}

approve_rollout_if_needed() {
  local rollout="${1:-${ROLLOUT}}"
  local phase current approved stage_name
  phase="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o jsonpath='{.status.phase}' 2>/dev/null || true)"
  [[ "${phase}" == "AwaitingApproval" ]] || return 0
  current="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o jsonpath='{.status.currentStage}')"
  approved="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o jsonpath='{.status.approvedStage}' 2>/dev/null || echo 0)"
  stage_name="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o jsonpath="{.spec.stages[${current}].name}" 2>/dev/null || echo "?")"
  if [[ "${approved}" -ge "${current}" ]]; then
    log "rollout stage ${current} (${stage_name}) already approved (approvedStage=${approved}); waiting for operator"
    return 0
  fi
  log "approving rollout stage ${current} (${stage_name})"
  if api_curl POST "/api/v1/rollouts/${NS_SYSTEM}/${rollout}/approve" \
    "{\"stage\":${current},\"actor\":\"e2e\"}" >/dev/null 2>&1; then
    return 0
  fi
  log "API approve unavailable; patching rollout status"
  kubectl_ctx patch rollout "${rollout}" -n "${NS_SYSTEM}" --subresource=status --type=merge -p \
    "{\"status\":{\"approvedStage\":${current}}}"
}

wait_rollout_rolled_back() {
  local rollout="${1:-${ROLLOUT}}"
  wait_for "rollout ${rollout} rolled back" -n "${NS_SYSTEM}" \
    --for=jsonpath='{.status.phase}=RolledBack' "rollout/${rollout}"
}

wait_until_rollout_stage_suffix() {
  local suffix="$1"
  local rollout="${2:-${ROLLOUT}}"
  local start
  start="$(date +%s)"
  while true; do
    local phase current name
    phase="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o jsonpath='{.status.phase}' 2>/dev/null || echo Pending)"
    case "${phase}" in
      RolledBack|Failed)
        kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o yaml || true
        die "rollback e2e: rollout reached ${phase} before verify injection point"
        ;;
      Completed)
        die "rollback e2e: rollout completed before verify injection (test invalid)"
        ;;
    esac
    current="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o jsonpath='{.status.currentStage}')"
    name="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o jsonpath="{.spec.stages[${current}].name}" 2>/dev/null || echo "")"
    if [[ "${name}" == *"${suffix}" ]]; then
      log "rollout ${rollout} at stage ${current} (${name})"
      return 0
    fi
    approve_rollout_if_needed "${rollout}"
    if (( "$(date +%s)" - start > E2E_TIMEOUT_SEC )); then
      kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o yaml || true
      die "timeout waiting for rollout ${rollout} stage *${suffix}"
    fi
    sleep 2
  done
}

inject_verify_failure() {
  log "removing ambient labels to force VerifyTraffic failure"
  kubectl_ctx label namespace "${BOOKINFO_NS}" \
    istio.io/dataplane-mode- istio.io/use-waypoint- --overwrite 2>/dev/null || true
}

assert_rollout_rollback_state() {
  local dp wp
  dp="$(kubectl_ctx get namespace "${BOOKINFO_NS}" -o jsonpath='{.metadata.labels.istio\.io/dataplane-mode}' 2>/dev/null || true)"
  [[ "${dp}" != "ambient" ]] || die "rollback: namespace still has dataplane-mode=ambient"
  wp="$(kubectl_ctx get namespace "${BOOKINFO_NS}" -o jsonpath='{.metadata.labels.istio\.io/use-waypoint}' 2>/dev/null || true)"
  [[ -z "${wp}" ]] || die "rollback: namespace still has use-waypoint=${wp}"
  if kubectl_ctx get gateway waypoint -n "${BOOKINFO_NS}" >/dev/null 2>&1; then
    die "rollback: waypoint Gateway still exists after rollback"
  fi
  log "rollback state OK (ambient labels and waypoint reverted)"
}

run_rollback_failure_e2e() {
  if [[ "${SKIP_ROLLBACK_E2E}" == "1" ]]; then
    log "SKIP rollback failure e2e (SKIP_ROLLBACK_E2E=1)"
    return 0
  fi
  log "rollback e2e: create rollout and inject verify failure"
  api_curl POST "/api/v1/plans/${NS_SYSTEM}/${PLAN}/rollout" '{}' >/dev/null
  approve_rollout_if_needed "${ROLLOUT}"
  wait_until_rollout_stage_suffix "-verify" "${ROLLOUT}"
  inject_verify_failure
  wait_rollout_rolled_back "${ROLLOUT}"
  assert_rollout_rollback_state
  log "deleting rolled-back rollout before happy-path retry"
  kubectl_ctx delete rollout "${ROLLOUT}" -n "${NS_SYSTEM}" --ignore-not-found --wait=true
}

wait_rollout_terminal() {
  local rollout="${1:-${ROLLOUT}}"
  local start
  start="$(date +%s)"
  while true; do
    local phase
    phase="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o jsonpath='{.status.phase}' 2>/dev/null || echo Pending)"
    case "${phase}" in
      Completed)
        log "rollout ${rollout} completed"
        return 0
        ;;
      Failed|RolledBack)
        kubectl_ctx logs -n "${NS_SYSTEM}" -l app=ambientor-operator --tail=100 || true
        kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o yaml || true
        local failed_msg failed_name
        failed_msg="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" \
          -o jsonpath='{range .status.stageResults[?(@.phase=="Failed")]}{.name}: {.message}{"\n"}{end}' 2>/dev/null || true)"
        failed_name="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" \
          -o jsonpath='{range .status.stageResults[?(@.phase=="Failed")]}{.name}{"\n"}{end}' 2>/dev/null | head -1)"
        if [[ -n "${failed_msg}" ]]; then
          die "rollout ${rollout} ended in phase ${phase} (failed stage: ${failed_msg})"
        fi
        die "rollout ${rollout} ended in phase ${phase} (last stage ${failed_name:-unknown})"
        ;;
    esac
    approve_rollout_if_needed "${rollout}"
    phase="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o jsonpath='{.status.phase}' 2>/dev/null || echo Pending)"
    if (( "$(date +%s)" - start > E2E_TIMEOUT_SEC )); then
      kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o yaml || true
      die "rollout ${rollout} timed out after ${E2E_TIMEOUT_SEC}s (phase=${phase}, currentStage=$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o jsonpath='{.status.currentStage}' 2>/dev/null || echo ?), approvedStage=$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${rollout}" -o jsonpath='{.status.approvedStage}' 2>/dev/null || echo ?))"
    fi
    sleep 5
  done
}

load_e2e_images() {
  local tag="${AMBIENTOR_IMAGE_TAG:-0.1.4}"
  local registry="${AMBIENTOR_IMAGE_REGISTRY:-}"
  local image component
  for component in operator api; do
    if [[ -n "${registry}" ]]; then
      image="${registry}/ambientor-${component}:${tag}"
    else
      image="ambientor-${component}:${tag}"
    fi
    if ! docker image inspect "${image}" >/dev/null 2>&1; then
      die "image ${image} not in local Docker; CI build must use load: true before kind load"
    fi
    log "loading ${image} into kind cluster ${CLUSTER}"
    kind load docker-image "${image}" --name "${CLUSTER}"
  done
}

api_curl() {
  local method="$1"
  local path="$2"
  local body="${3:-}"
  kubectl_ctx port-forward -n "${NS_SYSTEM}" svc/ambientor-api 18080:8080 >/dev/null 2>&1 &
  local pf_pid=$!
  sleep 3
  if [[ -n "${body}" ]]; then
    curl -sf -X "${method}" -H "Content-Type: application/json" -d "${body}" \
      "http://127.0.0.1:18080${path}"
  else
    curl -sf -X "${method}" "http://127.0.0.1:18080${path}"
  fi
  local rc=$?
  kill "${pf_pid}" 2>/dev/null || true
  wait "${pf_pid}" 2>/dev/null || true
  return "${rc}"
}

if [[ "${SKIP_CLUSTER_CREATE}" != "1" ]]; then
  if kind get clusters 2>/dev/null | grep -qx "${CLUSTER}"; then
    log "reusing existing kind cluster ${CLUSTER}"
  else
    log "creating kind cluster ${CLUSTER}"
    kind create cluster --name "${CLUSTER}" --config docs/lab/kind-config-e2e.yaml
  fi
fi

kubectl_ctx cluster-info

if ! command -v istioctl >/dev/null 2>&1; then
  die "istioctl not found; install Istio ${ISTIO_VERSION} CLI"
fi

log "installing Istio ${ISTIO_VERSION} ambient profile"
istioctl install --set profile=ambient -y --context "${CTX}"

log "installing Gateway API CRDs ${GATEWAY_API_VERSION}"
kubectl_ctx apply -f "https://github.com/kubernetes-sigs/gateway-api/releases/download/${GATEWAY_API_VERSION}/standard-install.yaml"

log "deploying minimal bookinfo (ratings only; sidecar mode)"
kubectl_ctx create namespace "${BOOKINFO_NS}" --dry-run=client -o yaml | kubectl_ctx apply -f -
kubectl_ctx label namespace "${BOOKINFO_NS}" istio-injection=enabled --overwrite
kubectl_ctx apply -n "${BOOKINFO_NS}" -f docs/lab/bookinfo-e2e.yaml
wait_for_pod_ready "bookinfo ratings" "${BOOKINFO_NS}" "app=ratings"

log "installing Ambientor CRDs"
kubectl_ctx apply -k config/crd/

if [[ "${SKIP_IMAGE_BUILD}" != "1" ]]; then
  ./scripts/lab-build-images.sh
fi
load_e2e_images

dump_ambientor_diagnostics() {
  log "diagnostics: pods and events in ${NS_SYSTEM}"
  kubectl_ctx get pods,events,deployments,statefulsets -n "${NS_SYSTEM}" || true
  kubectl_ctx describe pods -n "${NS_SYSTEM}" || true
}

install_ambientor() {
  log "installing Ambientor Helm chart (e2e values, no Postgres)"
  helm dependency update deploy/helm/ambientor/
  if ! helm upgrade --install ambientor deploy/helm/ambientor/ \
    -n "${NS_SYSTEM}" --create-namespace \
    -f deploy/helm/ambientor/values-e2e.yaml \
    --kube-context "${CTX}" \
    --timeout 15m; then
    dump_ambientor_diagnostics
    die "helm install failed"
  fi
  wait_for_deployment "ambientor operator" "${NS_SYSTEM}" "${AMBIENTOR_OPERATOR_DEPLOY}"
  wait_for_deployment "ambientor api" "${NS_SYSTEM}" "${AMBIENTOR_API_DEPLOY}"
}

install_ambientor

log "triggering mesh inventory scan"
kubectl_ctx apply -f docs/lab/meshinventory-bookinfo.yaml

wait_for "assessment ${ASSESSMENT}" -n "${NS_SYSTEM}" \
  --for=jsonpath="{.status.phase}=Completed" "ambientassessment/${ASSESSMENT}"

wait_for "migration plan ${PLAN}" -n "${NS_SYSTEM}" \
  --for=jsonpath="{.status.phase}=Ready" "migrationplan/${PLAN}"

plan_ns="$(kubectl_ctx get migrationplan -n "${NS_SYSTEM}" "${PLAN}" \
  -o jsonpath='{.spec.waves[0].namespaces[0]}' 2>/dev/null || true)"
log "migration plan wave-1 namespace: ${plan_ns:-unknown}"
[[ "${plan_ns}" == "${BOOKINFO_NS}" ]] || die "expected plan to target ${BOOKINFO_NS}, got '${plan_ns}'"

run_rollback_failure_e2e

log "creating rollout from plan via API (happy path)"
api_curl POST "/api/v1/plans/${NS_SYSTEM}/${PLAN}/rollout" '{}' >/dev/null

log "driving rollout to completion (approving gated stages)"
wait_rollout_terminal

log "verifying bookinfo namespace ambient enrollment"
dataplane="$(kubectl_ctx get namespace "${BOOKINFO_NS}" -o jsonpath='{.metadata.labels.istio\.io/dataplane-mode}' 2>/dev/null || true)"
[[ "${dataplane}" == "ambient" ]] || die "expected bookinfo dataplane-mode=ambient, got '${dataplane}'"

if command -v jq >/dev/null 2>&1; then
  log "checking audit log (optional without Postgres in e2e)"
  if audit_json="$(api_curl GET "/api/v1/rollouts/${NS_SYSTEM}/${ROLLOUT}/audit?limit=10" 2>/dev/null)"; then
    audit_count="$(echo "${audit_json}" | jq 'length')"
    log "audit events: ${audit_count}"
  else
    log "audit API unavailable (expected when Postgres disabled in e2e)"
  fi
fi

log "e2e passed: rollback on verify failure + bookinfo → assessment → plan → rollout → ambient namespace"
