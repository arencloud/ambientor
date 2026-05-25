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
KUBECTL_TIMEOUT="${PILOT_KUBECTL_TIMEOUT:-15s}"

log() { echo "[pilot] $*"; }
die() { log "ERROR: $*"; exit 1; }

mkdir -p "${RUN_DIR}"

if [[ -n "${PILOT_CONTEXTS:-}" ]]; then
  mapfile -t CONTEXTS < <(echo "${PILOT_CONTEXTS}" | tr ',' '\n')
elif [[ -f "${CONTEXTS_FILE}" ]]; then
  mapfile -t CONTEXTS < <(grep -v '^[[:space:]]*#' "${CONTEXTS_FILE}" | grep -v '^[[:space:]]*$' || true)
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

slugify() {
  echo "$1" | tr '/: ' '___' | sed 's/[^a-zA-Z0-9._-]/_/g' | cut -c1-80
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

  if ! kubectl --context "${ctx}" --request-timeout="${KUBECTL_TIMEOUT}" cluster-info >/dev/null 2>&1; then
    log "SKIP unreachable: ${ctx}"
    echo "| ${slug} | \`${ctx}\` | ⬜ | — | — | — | unreachable from this host |" >>"${SIGNOFF}"
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
    P2_OK=$((P2_OK + 1))
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
- P2 clusters with ≥1 plan export: **${P2_OK}** / ${#CONTEXTS[@]} (need ≥1 total)

Update [PROGRESS.md](../docs/PROGRESS.md) when P1 ≥3 and P2 ≥1 are satisfied.
EOF

log "Wrote ${SIGNOFF}"
log "P1 ok=${P1_OK} fail=${P1_FAIL}  P2 ok=${P2_OK} fail=${P2_FAIL}"

if [[ "${P1_OK}" -ge 3 && "${P2_OK}" -ge 1 ]]; then
  log "Pilot criteria met — update PROGRESS.md P1/P2 to ✅"
  exit 0
fi

log "Pilot criteria not fully met (need 3+ P1 OK and 1+ P2 export)"
exit 2
