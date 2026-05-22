#!/usr/bin/env bash
# Load lab images into a kind cluster.
set -euo pipefail
CLUSTER="${1:-ambientor-lab}"
TAG="${AMBIENTOR_IMAGE_TAG:-0.1.0}"
REPO="${AMBIENTOR_IMAGE_REPO:-ambientor}"

for suffix in operator api web; do
  echo "Loading ${REPO}:${TAG}-${suffix} -> kind cluster ${CLUSTER}"
  kind load docker-image "${REPO}:${TAG}-${suffix}" --name "${CLUSTER}"
done
echo "Done."
