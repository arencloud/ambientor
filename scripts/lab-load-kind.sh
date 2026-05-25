#!/usr/bin/env bash
# Load lab images into a kind cluster.
set -euo pipefail
CLUSTER="${1:-ambientor-lab}"
TAG="${AMBIENTOR_IMAGE_TAG:-0.1.0}"
REGISTRY="${AMBIENTOR_IMAGE_REGISTRY:-}"

image_ref() {
  local component="$1"
  if [[ -n "${REGISTRY}" ]]; then
    echo "${REGISTRY}/ambientor-${component}:${TAG}"
  else
    echo "ambientor-${component}:${TAG}"
  fi
}

for component in operator api web; do
  image="$(image_ref "${component}")"
  echo "Loading ${image} -> kind cluster ${CLUSTER}"
  kind load docker-image "${image}" --name "${CLUSTER}"
done
echo "Done."
