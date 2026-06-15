#!/usr/bin/env bash
# Create a selection-based MigrationPlan for P2 pilot sign-off (kubectl + operator reconcile).
set -euo pipefail

CONTEXT="${1:-}"
NAMESPACE="${2:-}"
ASSESSMENT_REF="${3:-}"
PLAN_NAME="${4:-pilot-selection-plan}"
AMBIENTOR_NS="${AMBIENTOR_NS:-ambientor-system}"

usage() {
  echo "Usage: $0 <kubectl-context> <target-namespace> <assessment-name> [plan-name]" >&2
  echo "Example: $0 pilot-cl01 bookinfo bookinfo-scan-assessment" >&2
  exit 1
}

[[ -n "${CONTEXT}" && -n "${NAMESPACE}" && -n "${ASSESSMENT_REF}" ]] || usage

CTX=(--context "${CONTEXT}")

if kubectl "${CTX[@]}" get migrationplan -n "${AMBIENTOR_NS}" "${PLAN_NAME}" >/dev/null 2>&1; then
  echo "MigrationPlan ${AMBIENTOR_NS}/${PLAN_NAME} already exists"
  kubectl "${CTX[@]}" get migrationplan -n "${AMBIENTOR_NS}" "${PLAN_NAME}" -o jsonpath='{.status.phase}{"\n"}'
  exit 0
fi

tmp="$(mktemp)"
trap 'rm -f "${tmp}"' EXIT
cat >"${tmp}" <<EOF
apiVersion: ambientor.io/v1alpha1
kind: MigrationPlan
metadata:
  name: ${PLAN_NAME}
  namespace: ${AMBIENTOR_NS}
spec:
  assessmentRef: ${ASSESSMENT_REF}
  selectedNamespaces:
    - ${NAMESPACE}
  clusterRef: in-cluster
  targetMeshMode: ambient
EOF

echo "Applying MigrationPlan ${AMBIENTOR_NS}/${PLAN_NAME} (namespace ${NAMESPACE})"
kubectl "${CTX[@]}" apply -f "${tmp}"
kubectl "${CTX[@]}" wait -n "${AMBIENTOR_NS}" --for=jsonpath='{.status.phase}=Ready' \
  "migrationplan/${PLAN_NAME}" --timeout=300s
echo "Plan ready: ${AMBIENTOR_NS}/${PLAN_NAME}"
