#!/usr/bin/env bash
# Run production pilot P1 + P2 across clusters listed in pilot-contexts.txt.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "${ROOT}"

CONTEXTS_FILE="${PILOT_CONTEXTS_FILE:-${ROOT}/scripts/pilot-contexts.txt}"
ARTIFACTS_ROOT="${PILOT_ARTIFACTS_ROOT:-${ROOT}/pilot-artifacts}"
AMBIENTOR="${AMBIENTOR:-${ROOT}/target/release/ambientor}"
DATE_TAG="$(date +%Y%m%d)"
RUN_DIR="${ARTIFACTS_ROOT}/${DATE_TAG}-validate"
SIGNOFF="${RUN_DIR}/PILOT-SIGNOFF.md"
KUBECTL_TIMEOUT="${PILOT_KUBECTL_TIMEOUT:-30s}"

log() { echo "[pilot] $*"; }
die() { log "ERROR: $*"; exit 1; }

mkdir -p "${RUN_DIR}"

if [[ -n "${PILOT_CONTEXTS:-}" ]]; then
  mapfile -t CONTEXTS < <(echo "${PILOT_CONTEXTS}" | tr ',' '\n')
elif [[ -f "${CONTEXTS_FILE}" ]]; then
  mapfile -t CONTEXTS < <(
    grep -v '^[[:space:]]*#' "${CONTEXTS_FILE}" | grep -v '^[[:space:]]*$' | sed 's/[[:space:]]*$//' || true
  )
else
  die "No contexts: set PILOT_CONTEXTS or create ${CONTEXTS_FILE} from scripts/pilot-contexts.example"
fi

[[ "${#CONTEXTS[@]}" -ge 3 ]] || die "P1 requires at least 3 contexts (got ${#CONTEXTS[@]})"

if [[ ! -x "${AMBIENTOR}" ]]; then
  log "Building ambientor CLI…"
  cargo build -p ambientor-cli --release
fi

command -v jq >/dev/null || die "jq is required"
command -v kubectl >/dev/null || die "kubectl is required"

P1_OK=0
P1_FAIL=0
P2_OK=0
P2_FAIL=0
P2_EXPORTS=0

slugify() {
  echo "$1" | tr '/: ' '___' | sed 's/[^a-zA-Z0-9._-]/_/g' | cut -c1-80
}

# Same API check kubectl uses; oc get nodes can work when cluster-info is slow.
cluster_reachable() {
  local ctx="$1"
  kubectl --context "${ctx}" --request-timeout="${KUBECTL_TIMEOUT}" get --raw=/healthz >/dev/null 2>&1 \
    || kubectl --context "${ctx}" --request-timeout="${KUBECTL_TIMEOUT}" cluster-info >/dev/null 2>&1
}

cat >"${SIGNOFF}" <<EOF
# Production pilot sign-off

Generated: $(date -u +%Y-%m-%dT%H:%M:%SZ)  
Artifacts: \`${RUN_DIR}\`

## P1 — Assessment blockers (3+ clusters)

| Cluster | Context | Blockers OK | Overall | Blockers | Warnings | Notes |
|---------|---------|-------------|---------|----------|----------|-------|
EOF

for ctx in "${CONTEXTS[@]}"; do
  slug="$(slugify "${ctx}")"
  out="${RUN_DIR}/${slug}"
  log "=== ${ctx} -> ${out} ==="

  if ! cluster_reachable "${ctx}"; then
    log "SKIP unreachable: ${ctx}"
    kubectl --context "${ctx}" --request-timeout="${KUBECTL_TIMEOUT}" cluster-info 2>&1 | head -3 \
      | sed 's/^/[pilot]   /' >&2 || true
    echo "| ${slug} | \`${ctx}\` | ⬜ | — | — | — | kubectl could not reach API (see log above) |" >>"${SIGNOFF}"
    P1_FAIL=$((P1_FAIL + 1))
    P2_FAIL=$((P2_FAIL + 1))
    continue
  fi

  "${ROOT}/scripts/pilot-record-assessment.sh" "${ctx}" "${out}" || die "record failed for ${ctx}"

  if [[ -f "${out}/assessment.json" ]]; then
    if "${ROOT}/scripts/pilot-analyze-blockers.sh" "${out}/assessment.json"; then
      P1_OK=$((P1_OK + 1))
      bcount="$(jq '[.findings[]? | select(.severity=="blocker")] | length' "${out}/assessment.json")"
      wcount="$(jq '[.findings[]? | select(.severity=="warning")] | length' "${out}/assessment.json")"
      overall="$(jq -r '.scores.overall // "—"' "${out}/assessment.json")"
      echo "| ${slug} | \`${ctx}\` | ✅ | ${overall} | ${bcount} | ${wcount} | |" >>"${SIGNOFF}"
    else
      P1_FAIL=$((P1_FAIL + 1))
      echo "| ${slug} | \`${ctx}\` | ❌ | — | — | — | unexpected blockers |" >>"${SIGNOFF}"
    fi
  else
    P1_FAIL=$((P1_FAIL + 1))
    echo "| ${slug} | \`${ctx}\` | ⬜ | — | — | — | no assessment.json (install Ambientor / run assess) |" >>"${SIGNOFF}"
  fi

  if "${ROOT}/scripts/pilot-export-plans.sh" "${ctx}" "${out}"; then
    exported="$(find "${out}/plans" -name '*.yaml' 2>/dev/null | wc -l | tr -d ' ')"
    P2_EXPORTS=$((P2_EXPORTS + exported))
    if [[ "${exported}" -gt 0 ]]; then
      P2_OK=$((P2_OK + 1))
    else
      P2_FAIL=$((P2_FAIL + 1))
    fi
  else
    P2_FAIL=$((P2_FAIL + 1))
  fi
done

cat >>"${SIGNOFF}" <<EOF

## P2 — Plan exports

| Cluster | Plans exported |
|---------|----------------|
EOF

for ctx in "${CONTEXTS[@]}"; do
  slug="$(slugify "${ctx}")"
  out="${RUN_DIR}/${slug}"
  count="$(find "${out}/plans" -name '*.yaml' 2>/dev/null | wc -l | tr -d ' ')"
  echo "| ${slug} | ${count} |" >>"${SIGNOFF}"
done

cat >>"${SIGNOFF}" <<EOF

## Summary

- P1 clusters with blockers OK: **${P1_OK}** / ${#CONTEXTS[@]} (need ≥3 for sign-off)
- P2 plan YAML exports (all clusters): **${P2_EXPORTS}** (need ≥1 total)
- P2 clusters that exported ≥1 plan: **${P2_OK}** / ${#CONTEXTS[@]}

Update [PROGRESS.md](../docs/PROGRESS.md): mark P1 ✅ when ≥3 clusters pass blockers; P2 ✅ when ≥1 plan export exists.
EOF

log "Wrote ${SIGNOFF}"
log "P1 ok=${P1_OK} fail=${P1_FAIL}  P2 exports=${P2_EXPORTS} clusters_with_plans=${P2_OK}"

if [[ "${P1_OK}" -ge 3 && "${P2_EXPORTS}" -ge 1 ]]; then
  log "Pilot criteria met — update PROGRESS.md P1 and P2 to ✅"
  exit 0
fi

if [[ "${P1_OK}" -ge 3 ]]; then
  log "P1 met (≥3 clusters). P2 still needs MigrationPlan CR(s) and re-run (or manual plan export)."
  exit 2
fi

log "Pilot criteria not fully met (need 3+ P1 OK and ≥1 plan YAML export)"
exit 2
