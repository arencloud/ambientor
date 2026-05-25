# Portal assessment dashboard (Phase 1, Step 1.7)

Branch: `feature/portal-assessment-dashboard`

## Goal

Wire the web portal to the API so operators can run assessments, browse CR status, and review findings with evidence.

## Delivered

- [x] `GET /api/v1/assessments` — list completed `AmbientAssessment` CRs from cluster
- [x] Dashboard: run assessment, score breakdown, findings with evidence
- [x] Assessments view: sidebar list + detail panel
- [x] SSE live event log on dashboard
- [x] `/config.js` injects `AMBIENTOR_API_URL` from Helm env

## Test plan

- [x] `cargo test --workspace`
- [ ] Port-forward API + web; run assessment and open Assessments tab

## Next

- Step 1.8 SARIF export
- Step 1.9 persist scans in Postgres
