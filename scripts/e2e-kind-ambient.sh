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
POD_READY_TIMEOUT_SEC="${POD_READY_TIMEOUT_SEC:-180}"
POLL_INTERVAL_SEC="${POLL_INTERVAL_SEC:-5}"
SKIP_CLUSTER_CREATE="${SKIP_CLUSTER_CREATE:-0}"
SKIP_IMAGE_BUILD="${SKIP_IMAGE_BUILD:-0}"

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

approve_rollout_if_needed() {
  local phase current
  phase="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${ROLLOUT}" -o jsonpath='{.status.phase}' 2>/dev/null || true)"
  [[ "${phase}" == "AwaitingApproval" ]] || return 0
  current="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${ROLLOUT}" -o jsonpath='{.status.currentStage}')"
  log "approving rollout stage ${current}"
  kubectl_ctx patch rollout "${ROLLOUT}" -n "${NS_SYSTEM}" --subresource=status --type=merge -p \
    "{\"status\":{\"approvedStage\":${current},\"phase\":\"Pending\"}}"
}

wait_rollout_terminal() {
  local start
  start="$(date +%s)"
  while true; do
    local phase
    phase="$(kubectl_ctx get rollout -n "${NS_SYSTEM}" "${ROLLOUT}" -o jsonpath='{.status.phase}' 2>/dev/null || echo Pending)"
    case "${phase}" in
      Completed)
        log "rollout completed"
        return 0
        ;;
      Failed|RolledBack)
        kubectl_ctx get rollout -n "${NS_SYSTEM}" "${ROLLOUT}" -o yaml || true
        die "rollout ended in phase ${phase}"
        ;;
    esac
    approve_rollout_if_needed
    if (( "$(date +%s)" - start > E2E_TIMEOUT_SEC )); then
      kubectl_ctx get rollout -n "${NS_SYSTEM}" "${ROLLOUT}" -o yaml || true
      die "rollout timed out after ${E2E_TIMEOUT_SEC}s (phase=${phase})"
    fi
    sleep 10
  done
}

load_e2e_images() {
  local tag="${AMBIENTOR_IMAGE_TAG:-0.1.0}"
  local repo="${AMBIENTOR_IMAGE_REPO:-ambientor}"
  local image
  for suffix in operator api; do
    image="${repo}:${tag}-${suffix}"
    if ! docker image inspect "${image}" >/dev/null 2>&1; then
      die "image ${image} not in local Docker; CI must use buildx driver: docker with load: true"
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
    kind create cluster --name "${CLUSTER}" --config docs/lab/kind-config.yaml
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

log "deploying bookinfo (sidecar mode)"
kubectl_ctx create namespace "${BOOKINFO_NS}" --dry-run=client -o yaml | kubectl_ctx apply -f -
kubectl_ctx label namespace "${BOOKINFO_NS}" istio-injection=enabled --overwrite
kubectl_ctx apply -n "${BOOKINFO_NS}" -f \
  "https://raw.githubusercontent.com/istio/istio/release-1.24/samples/bookinfo/platform/kube/bookinfo.yaml"
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
  wait_for_pod_ready "ambientor operator" "${NS_SYSTEM}" "app=ambientor-operator"
  wait_for_pod_ready "ambientor api" "${NS_SYSTEM}" "app=ambientor-api"
}

install_ambientor

log "triggering mesh inventory scan"
kubectl_ctx apply -f docs/lab/meshinventory-bookinfo.yaml

wait_for "assessment ${ASSESSMENT}" -n "${NS_SYSTEM}" \
  --for=jsonpath="{.status.phase}=Completed" "ambientassessment/${ASSESSMENT}"

wait_for "migration plan ${PLAN}" -n "${NS_SYSTEM}" \
  --for=jsonpath="{.status.phase}=Ready" "migrationplan/${PLAN}"

log "creating rollout from plan via API"
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

log "e2e passed: bookinfo → assessment → plan → rollout → ambient namespace"
