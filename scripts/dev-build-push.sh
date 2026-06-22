#!/usr/bin/env bash
# Build and push Ambientor *-dev images with a unique tag every run.
#
# Why not only dev-$(git rev-parse --short HEAD)?
# - Same tag until you commit → cluster may not pull new layers (imagePullPolicy: IfNotPresent).
# - This script appends UTC time (and -dirty when the tree is not clean).
#
# Usage:
#   ./scripts/dev-build-push.sh
#   ./scripts/dev-build-push.sh --helm-upgrade   # upgrade images on current context (reuses release values when present)
#   API_URL=https://ambientor-api-.... ./scripts/dev-build-push.sh --helm-upgrade
#
set -euo pipefail
cd "$(dirname "$0")/.."

REGISTRY="${AMBIENTOR_DEV_REGISTRY:-quay.io/arencloud}"
HELM_UPGRADE=false
for arg in "$@"; do
  case "$arg" in
    --helm-upgrade) HELM_UPGRADE=true ;;
    -h | --help)
      sed -n '2,12p' "$0"
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg" >&2
      exit 1
      ;;
  esac
done

SHA="$(git rev-parse --short HEAD)"
DIRTY=""
if ! git diff --quiet --ignore-submodules 2>/dev/null \
  || ! git diff --cached --quiet --ignore-submodules 2>/dev/null; then
  DIRTY="-dirty"
fi
TS="$(date -u +%Y%m%d%H%M%S)"
TAG="dev-${SHA}${DIRTY}-${TS}"

echo "Image tag: ${TAG}"
echo "  (git: ${SHA}${DIRTY}, time: ${TS})"
echo "${TAG}" >.dev-image-tag

build_push() {
  local target="$1"
  local repo="${REGISTRY}/ambientor-${target}-dev"
  echo "==> ${target}: ${repo}:${TAG}"
  podman build --target "${target}" -t "${repo}:${TAG}" .
  podman push "${repo}:${TAG}"
}

for t in operator api web cli; do
  build_push "${t}"
done

echo ""
echo "Built and pushed. Tag written to .dev-image-tag"
echo ""
echo "Helm upgrade example:"
cat <<EOF
helm upgrade --install ambientor deploy/helm/ambientor/ \\
  -n ambientor-system --create-namespace \\
  --set image.pullPolicy=Always \\
  --set operator.image.repository=${REGISTRY}/ambientor-operator-dev \\
  --set operator.image.tag=${TAG} \\
  --set api.image.repository=${REGISTRY}/ambientor-api-dev \\
  --set api.image.tag=${TAG} \\
  --set web.image.repository=${REGISTRY}/ambientor-web-dev \\
  --set web.image.tag=${TAG} \\
  --set openshift.apiUrl="\${API_URL:-https://ambientor-api-ambientor-system.apps.cl01.arencloud.com}"
EOF

if [[ "${HELM_UPGRADE}" == true ]]; then
  AMBIENTOR_DEPLOY_CONTEXT="${AMBIENTOR_DEPLOY_CONTEXT:-ambientor-system/api-cl01-arencloud-com:6443/egevorky}"
  if command -v kubectl >/dev/null 2>&1; then
    CURRENT_CTX="$(kubectl config current-context 2>/dev/null || true)"
    if [[ "${CURRENT_CTX}" != "${AMBIENTOR_DEPLOY_CONTEXT}" ]]; then
      echo "Switching kubectl context for deploy: ${CURRENT_CTX:-<none>} -> ${AMBIENTOR_DEPLOY_CONTEXT}"
      kubectl config use-context "${AMBIENTOR_DEPLOY_CONTEXT}"
    fi
  fi
  API_URL="${API_URL:-}"
  HELM_REUSE=()
  if helm status ambientor -n ambientor-system >/dev/null 2>&1; then
    HELM_REUSE+=(--reuse-values)
  fi
  API_URL_SET=()
  if command -v oc >/dev/null 2>&1; then
    API_HOST="$(oc get route ambientor-ambientor-api -n ambientor-system -o jsonpath='{.spec.host}' 2>/dev/null || true)"
    if [[ -n "${API_HOST}" ]]; then
      API_URL="https://${API_HOST}"
      API_URL_SET+=(--set "openshift.apiUrl=${API_URL}")
    fi
  fi
  helm upgrade --install ambientor deploy/helm/ambientor/ \
    -n ambientor-system --create-namespace \
    "${HELM_REUSE[@]}" \
    -f deploy/helm/ambientor/values-openshift-dev.yaml \
    --set image.pullPolicy=Always \
    --set operator.image.repository="${REGISTRY}/ambientor-operator-dev" \
    --set operator.image.tag="${TAG}" \
    --set api.image.repository="${REGISTRY}/ambientor-api-dev" \
    --set api.image.tag="${TAG}" \
    --set web.image.repository="${REGISTRY}/ambientor-web-dev" \
    --set web.image.tag="${TAG}" \
    --set openshift.routes.enabled=true \
    --set postgresql.primary.persistence.enabled=false \
    "${API_URL_SET[@]}"
  if command -v oc >/dev/null 2>&1; then
    WEB_HOST="$(oc get route ambientor-ambientor-web -n ambientor-system -o jsonpath='{.spec.host}' 2>/dev/null || true)"
    if [[ -n "${WEB_HOST}" ]]; then
      echo "Portal: https://${WEB_HOST}/"
    fi
    if [[ -n "${API_URL}" ]]; then
      echo "API: ${API_URL}/healthz"
    fi
  elif [[ -n "${API_URL}" ]]; then
    helm upgrade ambientor deploy/helm/ambientor/ -n ambientor-system --reuse-values \
      --set "openshift.apiUrl=${API_URL}"
  fi
  kubectl rollout restart deployment -n ambientor-system \
    -l 'app in (ambientor-operator,ambientor-api,ambientor-web)' 2>/dev/null || true
fi
