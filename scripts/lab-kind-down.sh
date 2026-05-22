#!/usr/bin/env bash
set -euo pipefail
CLUSTER="${AMBIENTOR_KIND_CLUSTER:-ambientor-lab}"
kind delete cluster --name "${CLUSTER}"
echo "Deleted kind cluster '${CLUSTER}'."
