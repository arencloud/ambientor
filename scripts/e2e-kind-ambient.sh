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
SKIP_CLUSTER_CREATE="${SKIP_CLUSTER_CREATE:-0}"
SKIP_IMAGE_BUILD="${SKIP_IMAGE_BUILD:-0}"

log() { echo "[e2e] $(date -u +%H:%M:%S) $*"; }
die() { log "ERROR: $*"; exit 1; }

kubectl_ctx() { kubectl --context "${CTX}" "$@"; }

wait_for() {
  local desc="$1"
  shift
  log "wait: ${desc}"
  kubectl_ctx wait "$@" --timeout="${E2E_TIMEOUT_SEC}s"
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
wait_for "bookinfo ratings ready" -n "${BOOKINFO_NS}" --for=condition=ready pod -l app=ratings

log "installing Ambientor CRDs"
kubectl_ctx apply -k config/crd/

if [[ "${SKIP_IMAGE_BUILD}" != "1" ]]; then
  ./scripts/lab-build-images.sh
  ./scripts/lab-load-kind.sh "${CLUSTER}"
fi

log "installing Ambientor Helm chart"
helm dependency update deploy/helm/ambientor/
helm upgrade --install ambientor deploy/helm/ambientor/ \
  -n "${NS_SYSTEM}" \
  -f deploy/helm/ambientor/values-lab.yaml \
  --kube-context "${CTX}" \
  --wait --timeout 10m

wait_for "ambientor pods" -n "${NS_SYSTEM}" --for=condition=ready pod -l app=ambientor-operator
wait_for "ambientor api" -n "${NS_SYSTEM}" --for=condition=ready pod -l app=ambientor-api

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
  log "checking audit log has rollout events"
  if audit_json="$(api_curl GET "/api/v1/rollouts/${NS_SYSTEM}/${ROLLOUT}/audit?limit=10" 2>/dev/null)"; then
    audit_count="$(echo "${audit_json}" | jq 'length')"
    [[ "${audit_count}" -gt 0 ]] || die "expected audit events, got ${audit_count}"
  else
    log "warn: could not query audit API (non-fatal)"
  fi
fi

log "e2e passed: bookinfo → assessment → plan → rollout → ambient namespace"
