#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
CLUSTER="${AMBIENTOR_KIND_CLUSTER:-ambientor-lab}"

if kind get clusters 2>/dev/null | grep -qx "${CLUSTER}"; then
  echo "kind cluster '${CLUSTER}' already exists"
  kubectl cluster-info --context "kind-${CLUSTER}"
  exit 0
fi

kind create cluster --name "${CLUSTER}" --config docs/lab/kind-config.yaml
echo "Cluster ready. Context: kind-${CLUSTER}"
kubectl cluster-info --context "kind-${CLUSTER}"
