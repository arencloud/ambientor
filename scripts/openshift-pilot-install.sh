#!/usr/bin/env bash
# Install Ambientor on OpenShift for pilot testing (Routes, Postgres, secrets, probes).
set -euo pipefail
cd "$(dirname "$0")/.."

NS="${AMBIENTOR_NS:-ambientor-system}"
RELEASE="${HELM_RELEASE:-ambientor}"
CHART="deploy/helm/ambientor"
VALUES="${CHART}/values-openshift-pilot.yaml"
TIMEOUT="${PILOT_INSTALL_TIMEOUT:-15m}"
JWT_SECRET="${AMBIENTOR_JWT_SECRET:-$(openssl rand -base64 32)}"
PG_PASSWORD="${AMBIENTOR_PG_PASSWORD:-$(openssl rand -base64 16)}"
STORAGE_CLASS="${AMBIENTOR_STORAGE_CLASS:-}"

log() { echo "[openshift-pilot] $*"; }
die() { log "ERROR: $*"; exit 1; }

command -v helm >/dev/null || die "helm required"
command -v oc >/dev/null 2>&1 || command -v kubectl >/dev/null || die "oc or kubectl required"

KUBE="${KUBECTL:-}"
if command -v oc >/dev/null 2>&1; then
  KUBE="oc"
elif command -v kubectl >/dev/null 2>&1; then
  KUBE="kubectl"
fi

log "target cluster: $($KUBE config current-context 2>/dev/null || echo unknown)"
log "namespace: ${NS}  release: ${RELEASE}"

helm_args=(
  upgrade --install "${RELEASE}" "${CHART}"
  -n "${NS}" --create-namespace
  -f "${VALUES}"
  --timeout "${TIMEOUT}"
  --set "auth.jwtSecret=${JWT_SECRET}"
  --set "postgresql.auth.password=${PG_PASSWORD}"
)

if [[ -n "${STORAGE_CLASS}" ]]; then
  helm_args+=(--set "postgresql.primary.persistence.storageClass=${STORAGE_CLASS}")
fi

if [[ -n "${AMBIENTOR_OIDC_ENABLED:-}" ]]; then
  helm_args+=(--set auth.oidc.enabled=true)
  [[ -n "${AMBIENTOR_OIDC_ISSUER_URL:-}" ]] && helm_args+=(--set "auth.oidc.issuerUrl=${AMBIENTOR_OIDC_ISSUER_URL}")
  [[ -n "${AMBIENTOR_OIDC_CLIENT_ID:-}" ]] && helm_args+=(--set "auth.oidc.clientId=${AMBIENTOR_OIDC_CLIENT_ID}")
  [[ -n "${AMBIENTOR_OIDC_CLIENT_SECRET:-}" ]] && helm_args+=(--set "auth.oidc.clientSecret=${AMBIENTOR_OIDC_CLIENT_SECRET}")
  [[ -n "${AMBIENTOR_OIDC_REDIRECT_URI:-}" ]] && helm_args+=(--set "auth.oidc.redirectUri=${AMBIENTOR_OIDC_REDIRECT_URI}")
fi

log "helm install (secrets written to cluster; save JWT and Postgres password if needed)"
helm dependency update "${CHART}" 2>/dev/null || true
helm "${helm_args[@]}" "$@"

API_DEPLOY="${RELEASE}-ambientor-api"
OP_DEPLOY="${RELEASE}-ambientor-operator"
WEB_DEPLOY="${RELEASE}-ambientor-web"

log "waiting for operator and API deployments"
$KUBE rollout status "deployment/${OP_DEPLOY}" -n "${NS}" --timeout="${TIMEOUT}"
$KUBE rollout status "deployment/${API_DEPLOY}" -n "${NS}" --timeout="${TIMEOUT}"
if $KUBE get deployment "${WEB_DEPLOY}" -n "${NS}" >/dev/null 2>&1; then
  $KUBE rollout status "deployment/${WEB_DEPLOY}" -n "${NS}" --timeout="${TIMEOUT}" || true
fi

API_ROUTE="${RELEASE}-ambientor-api"
WEB_ROUTE="${RELEASE}-ambientor-web"
if $KUBE get route "${API_ROUTE}" -n "${NS}" >/dev/null 2>&1; then
  API_HOST="$($KUBE get route "${API_ROUTE}" -n "${NS}" -o jsonpath='{.spec.host}')"
  WEB_HOST="$($KUBE get route "${WEB_ROUTE}" -n "${NS}" -o jsonpath='{.spec.host}')"
  log "routes: API https://${API_HOST}  Web https://${WEB_HOST}"
  log "patching web API URL for browser"
  helm upgrade "${RELEASE}" "${CHART}" -n "${NS}" --reuse-values \
    --set "openshift.apiUrl=https://${API_HOST}" \
    --set "auth.oidc.successUrl=https://${WEB_HOST}/" \
    --timeout "${TIMEOUT}"
  $KUBE rollout status "deployment/${WEB_DEPLOY}" -n "${NS}" --timeout="${TIMEOUT}" || true
  echo ""
  echo "Portal:  https://${WEB_HOST}/"
  echo "API:     https://${API_HOST}/healthz"
  if [[ -z "${AMBIENTOR_OIDC_REDIRECT_URI:-}" ]] && [[ -n "${AMBIENTOR_OIDC_ENABLED:-}" ]]; then
    echo "OIDC redirect URI (register in IdP): https://${API_HOST}/api/v1/auth/oidc/callback"
  fi
  echo ""
  echo "Next: ./scripts/openshift-pilot-smoke.sh"
  echo "      docs/runbook-openshift-pilot.md"
else
  log "no OpenShift Routes found; use port-forward or set openshift.routes.enabled"
  echo "  kubectl port-forward -n ${NS} svc/ambientor-api 8080:8080"
fi

# Save non-secret install metadata for the test session (gitignored path).
META_DIR="${PILOT_META_DIR:-./pilot-artifacts/openshift-install}"
mkdir -p "${META_DIR}"
cat >"${META_DIR}/install.env" <<EOF
AMBIENTOR_NS=${NS}
AMBIENTOR_RELEASE=${RELEASE}
AMBIENTOR_API_ROUTE=${API_ROUTE:-}
AMBIENTOR_WEB_ROUTE=${WEB_ROUTE:-}
AMBIENTOR_API_URL=${API_HOST:+https://${API_HOST}}
AMBIENTOR_WEB_URL=${WEB_HOST:+https://${WEB_HOST}}
EOF
log "wrote ${META_DIR}/install.env"
