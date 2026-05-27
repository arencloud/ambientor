#!/usr/bin/env bash
# Build Ambientor images for local kind load (tags match Helm: ambientor-<component>:<tag>).
set -euo pipefail
cd "$(dirname "$0")/.."

TAG="${AMBIENTOR_IMAGE_TAG:-0.1.2}"
REGISTRY="${AMBIENTOR_IMAGE_REGISTRY:-}"

image_ref() {
  local component="$1"
  if [[ -n "${REGISTRY}" ]]; then
    echo "${REGISTRY}/ambientor-${component}:${TAG}"
  else
    echo "ambientor-${component}:${TAG}"
  fi
}

echo "Building operator, api, web, cli ..."
docker build -t "$(image_ref operator)" --target operator .
docker build -t "$(image_ref api)" --target api .
docker build -t "$(image_ref web)" --target web .
docker build -t "$(image_ref cli)" --target cli .
echo "Done."
