#!/usr/bin/env bash
# Build Ambientor images for local kind load (tags match Helm: repo:tag-operator|api|web).
set -euo pipefail
cd "$(dirname "$0")/.."

TAG="${AMBIENTOR_IMAGE_TAG:-0.1.0}"
REPO="${AMBIENTOR_IMAGE_REPO:-ambientor}"

echo "Building ${REPO}:${TAG}-{operator,api,web,cli} ..."
docker build -t "${REPO}:${TAG}-operator" --target operator .
docker build -t "${REPO}:${TAG}-api" --target api .
docker build -t "${REPO}:${TAG}-web" --target web .
docker build -t "${REPO}:${TAG}-cli" --target cli .
echo "Done."
