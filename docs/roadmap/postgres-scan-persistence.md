# Postgres scan persistence (Phase 1, Step 1.9)

Branch: `cursor/postgres-scan-persistence`

## Goal

Persist assessment runs when `DATABASE_URL` is set (Helm Postgres subchart or external DB).

## Delivered

- [x] `ScanRepository` — insert/list on existing `scan_runs` table
- [x] API `POST /api/v1/assess` records completed scan (non-fatal on DB error)
- [x] API `GET /api/v1/scans` — historical runs (503 without `DATABASE_URL`)
- [x] Operator `AmbientAssessment` reconcile persists scan with `source: operator`
- [x] `AMBIENTOR_CLUSTER_REF` env (default `in-cluster`) for multi-cluster

## Test plan

- [x] `cargo test -p ambientor-db`
- [x] `cargo test --workspace`
- [ ] Lab: enable Postgres in Helm, run assess, `curl /api/v1/scans`
