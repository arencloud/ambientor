#!/usr/bin/env bash
# Summarize assessment blockers for production pilot P1 sign-off.
set -euo pipefail

ASSESSMENT_JSON="${1:-}"
ALLOWLIST="${PILOT_BLOCKER_ALLOWLIST:-}"

if [[ -z "${ASSESSMENT_JSON}" || ! -f "${ASSESSMENT_JSON}" ]]; then
  echo "Usage: $0 <assessment.json>  (optional PILOT_BLOCKER_ALLOWLIST=file)" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "jq is required" >&2
  exit 1
fi

BLOCKERS="$(jq -r '[.findings[]? | select(.severity == "blocker") | .title] | unique | .[]' "${ASSESSMENT_JSON}" 2>/dev/null || true)"
BLOCKER_COUNT="$(jq '[.findings[]? | select(.severity == "blocker")] | length' "${ASSESSMENT_JSON}")"
WARN_COUNT="$(jq '[.findings[]? | select(.severity == "warning")] | length' "${ASSESSMENT_JSON}")"
OVERALL="$(jq -r '.scores.overall // "—"' "${ASSESSMENT_JSON}")"

echo "overall_score: ${OVERALL}"
echo "blockers: ${BLOCKER_COUNT}"
echo "warnings: ${WARN_COUNT}"

if [[ -n "${BLOCKERS}" ]]; then
  echo "blocker_titles:"
  while IFS= read -r title; do
    [[ -z "${title}" ]] && continue
    echo "  - ${title}"
  done <<<"${BLOCKERS}"
fi

UNEXPECTED=0
if [[ "${BLOCKER_COUNT}" -gt 0 ]]; then
  if [[ -z "${ALLOWLIST}" || ! -f "${ALLOWLIST}" ]]; then
    UNEXPECTED="${BLOCKER_COUNT}"
  else
    while IFS= read -r title; do
      [[ -z "${title}" ]] && continue
      if ! grep -Fxq "${title}" "${ALLOWLIST}" 2>/dev/null; then
        echo "unexpected_blocker: ${title}" >&2
        UNEXPECTED=$((UNEXPECTED + 1))
      fi
    done <<<"${BLOCKERS}"
  fi
fi

if [[ "${BLOCKER_COUNT}" -eq 0 ]]; then
  echo "p1_blockers_ok: yes"
  exit 0
fi

if [[ "${UNEXPECTED}" -eq 0 ]]; then
  echo "p1_blockers_ok: yes (all blockers allowlisted)"
  exit 0
fi

echo "p1_blockers_ok: no (${UNEXPECTED} unexpected)" >&2
exit 1
