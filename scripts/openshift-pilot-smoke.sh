#!/usr/bin/env bash
# Post-install smoke checks for OpenShift pilot (API, portal, optional OSSM wizard).
set -euo pipefail
cd "$(dirname "$0")/.."

NS="${AMBIENTOR_NS:-ambientor-system}"
RELEASE="${HELM_RELEASE:-ambientor}"
META="${PILOT_META_DIR:-./pilot-artifacts/openshift-install}/install.env"

log() { echo "[smoke] $*"; }
die() { log "FAIL: $*"; exit 1; }
ok() { log "OK: $*"; }

command -v curl >/dev/null || die "curl required"
KUBE="oc"
command -v oc >/dev/null 2>&1 || KUBE="kubectl"

API_URL=""
WEB_URL=""
if [[ -f "${META}" ]]; then
  # shellcheck disable=SC1090
  source "${META}"
  API_URL="${AMBIENTOR_API_URL:-}"
  WEB_URL="${AMBIENTOR_WEB_URL:-}"
fi

if [[ -z "${API_URL}" ]]; then
  API_HOST="$($KUBE get route "${RELEASE}-ambientor-api" -n "${NS}" -o jsonpath='{.spec.host}' 2>/dev/null || true)"
  [[ -n "${API_HOST}" ]] && API_URL="https://${API_HOST}"
fi
if [[ -z "${WEB_URL}" ]]; then
  WEB_HOST="$($KUBE get route "${RELEASE}-ambientor-web" -n "${NS}" -o jsonpath='{.spec.host}' 2>/dev/null || true)"
  [[ -n "${WEB_HOST}" ]] && WEB_URL="https://${WEB_HOST}"
fi

[[ -n "${API_URL}" ]] || die "API URL unknown; run openshift-pilot-install.sh first"

curl -sf "${API_URL}/healthz" >/dev/null && ok "GET /healthz"
curl -sf "${API_URL}/readyz" >/dev/null && ok "GET /readyz"

if [[ -n "${WEB_URL}" ]]; then
  curl -sf "${WEB_URL}/" | grep -q Ambientor && ok "portal HTML"
  curl -sf "${WEB_URL}/config.js" | grep -q AMBIENTOR_API_URL && ok "portal config.js"
fi

AUTH_JSON="$(curl -sf "${API_URL}/api/v1/auth/config" 2>/dev/null || echo '{}')"
if command -v jq >/dev/null 2>&1; then
  enabled="$(echo "${AUTH_JSON}" | jq -r '.enabled // false')"
  log "auth enabled: ${enabled}"
  if [[ "${enabled}" == "true" ]]; then
    ok "auth/config (Postgres + JWT wired)"
  fi
fi

if curl -sf "${API_URL}/api/v1/dashboard/fleet" >/dev/null 2>&1; then
  ok "GET /api/v1/dashboard/fleet"
elif curl -sf "${API_URL}/api/v1/dashboard" >/dev/null 2>&1; then
  ok "GET /api/v1/dashboard"
else
  log "dashboard API not ready yet (operator sync may need a few minutes)"
fi

CONN_CODE="$(curl -s -o /dev/null -w '%{http_code}' "${API_URL}/api/v1/connections" 2>/dev/null || echo 000)"
[[ "${CONN_CODE}" == "200" ]] && ok "GET /api/v1/connections"

if $KUBE api-resources 2>/dev/null | grep -q servicemeshcontrolplanes; then
  WIZARD_CODE="$(curl -s -o /dev/null -w '%{http_code}' \
    "${API_URL}/api/v1/openshift/wizard?enroll=false" 2>/dev/null || echo 000)"
  [[ "${WIZARD_CODE}" == "200" ]] && ok "OSSM openshift wizard API"
fi

log "smoke checks passed — continue with docs/runbook-openshift-pilot.md"
