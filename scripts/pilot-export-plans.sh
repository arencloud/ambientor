#!/usr/bin/env bash
# Export all MigrationPlan resources for production pilot P2.
set -euo pipefail

CONTEXT="${1:-}"
OUT_DIR="${2:-}"

if [[ -z "${CONTEXT}" || -z "${OUT_DIR}" ]]; then
  echo "Usage: $0 <kubectl-context> <output-dir>" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
AMBIENTOR="${AMBIENTOR:-${ROOT}/target/release/ambientor}"
KUBECONFIG_SNIP="${OUT_DIR}/kubeconfig"
CTX=(--context "${CONTEXT}")

mkdir -p "${OUT_DIR}/plans"
kubectl config view --minify --flatten "${CTX[@]}" >"${KUBECONFIG_SNIP}"

if [[ ! -x "${AMBIENTOR}" ]]; then
  echo "ambientor CLI not found at ${AMBIENTOR}; build with: cargo build -p ambientor-cli --release" >&2
  exit 1
fi

mapfile -t PLANS < <(
  kubectl "${CTX[@]}" get migrationplan -A -o json 2>/dev/null \
    | jq -r '.items[]? | "\(.metadata.namespace)\t\(.metadata.name)"' || true
)

if [[ "${#PLANS[@]}" -eq 0 ]]; then
  echo "No MigrationPlan resources found on ${CONTEXT}" >&2
  exit 0
fi

EXPORTED=0
for row in "${PLANS[@]}"; do
  [[ -z "${row}" ]] && continue
  ns="${row%%$'\t'*}"
  name="${row#*$'\t'}"
  dest="${OUT_DIR}/plans/${ns}-${name}.yaml"
  echo "==> export ${ns}/${name} -> ${dest}"
  "${AMBIENTOR}" --kubeconfig "${KUBECONFIG_SNIP}" plan export -n "${ns}" --name "${name}" -o "${dest}"
  EXPORTED=$((EXPORTED + 1))
done

echo "exported_plans: ${EXPORTED}"
[[ "${EXPORTED}" -gt 0 ]]
