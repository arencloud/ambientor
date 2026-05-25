#!/usr/bin/env bash
# Record assessment artifacts for production pilot P1 (multi-cluster comparison).
set -euo pipefail

CONTEXT="${1:-}"
OUT_DIR="${2:-./pilot-artifacts/$(date +%Y%m%d-%H%M%S)}"

if [[ -z "${CONTEXT}" ]]; then
  echo "Usage: $0 <kubectl-context> [output-dir]" >&2
  exit 1
fi

mkdir -p "${OUT_DIR}"

export KUBECONFIG="${KUBECONFIG:-}"
CTX=(--context "${CONTEXT}")

echo "==> Cluster info -> ${OUT_DIR}/cluster.txt"
{
  kubectl "${CTX[@]}" version -o yaml 2>/dev/null || true
  kubectl "${CTX[@]}" get ns istio-system -o jsonpath='{.metadata.name}' 2>/dev/null && echo
  kubectl "${CTX[@]}" -n istio-system get deploy istiod -o jsonpath='{.spec.template.spec.containers[0].image}' 2>/dev/null && echo
} >"${OUT_DIR}/cluster.txt"

if command -v ambientor >/dev/null 2>&1; then
  echo "==> ambientor assess (json + sarif)"
  ambientor assess --output json >"${OUT_DIR}/assessment.json"
  ambientor assess --output sarif >"${OUT_DIR}/assessment.sarif"
else
  echo "==> ambientor CLI not in PATH; skipping CLI assess (use API or install CLI)" >&2
fi

echo "==> AmbientAssessment CRs"
kubectl "${CTX[@]}" get ambientassessment -A -o yaml >"${OUT_DIR}/ambientassessments.yaml" 2>/dev/null || true

echo "Wrote pilot artifacts to ${OUT_DIR}"
