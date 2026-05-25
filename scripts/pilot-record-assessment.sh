#!/usr/bin/env bash
# Record assessment artifacts for production pilot P1 (multi-cluster comparison).
set -euo pipefail

CONTEXT="${1:-}"
OUT_DIR="${2:-./pilot-artifacts/$(date +%Y%m%d-%H%M%S)}"

if [[ -z "${CONTEXT}" ]]; then
  echo "Usage: $0 <kubectl-context> [output-dir]" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
AMBIENTOR="${AMBIENTOR:-${ROOT}/target/release/ambientor}"
KUBECONFIG_SNIP="${OUT_DIR}/kubeconfig"
KUBECTL_TIMEOUT="${PILOT_KUBECTL_TIMEOUT:-30s}"

mkdir -p "${OUT_DIR}"

CTX=(--context "${CONTEXT}")
kubectl config view --minify --flatten "${CTX[@]}" >"${KUBECONFIG_SNIP}"

echo "==> Cluster info -> ${OUT_DIR}/cluster.txt"
{
  kubectl "${CTX[@]}" --request-timeout="${KUBECTL_TIMEOUT}" version -o yaml 2>/dev/null || true
  kubectl "${CTX[@]}" --request-timeout="${KUBECTL_TIMEOUT}" get ns istio-system -o jsonpath='{.metadata.name}' 2>/dev/null && echo
  kubectl "${CTX[@]}" --request-timeout="${KUBECTL_TIMEOUT}" -n istio-system get deploy istiod -o jsonpath='{.spec.template.spec.containers[0].image}' 2>/dev/null && echo
} >"${OUT_DIR}/cluster.txt"

if [[ -x "${AMBIENTOR}" ]]; then
  echo "==> ambientor assess (json + sarif)"
  "${AMBIENTOR}" --kubeconfig "${KUBECONFIG_SNIP}" assess --output json >"${OUT_DIR}/assessment.json"
  "${AMBIENTOR}" --kubeconfig "${KUBECONFIG_SNIP}" assess --output sarif >"${OUT_DIR}/assessment.sarif"
elif command -v ambientor >/dev/null 2>&1; then
  echo "==> ambientor assess (json + sarif)"
  ambientor --kubeconfig "${KUBECONFIG_SNIP}" assess --output json >"${OUT_DIR}/assessment.json"
  ambientor --kubeconfig "${KUBECONFIG_SNIP}" assess --output sarif >"${OUT_DIR}/assessment.sarif"
else
  echo "==> ambientor CLI not found; build with: cargo build -p ambientor-cli --release" >&2
fi

echo "==> AmbientAssessment CRs"
kubectl "${CTX[@]}" --request-timeout="${KUBECTL_TIMEOUT}" get ambientassessment -A -o yaml >"${OUT_DIR}/ambientassessments.yaml" 2>/dev/null || true

echo "Wrote pilot artifacts to ${OUT_DIR}"
