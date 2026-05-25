# Portal plan review + export (Phase 2, Step 2.3)

Branch: `feature/portal-plan-review`

## Goal

Review `MigrationPlan` waves and `PolicyTranslation` manifests in the portal; export a GitOps YAML bundle.

## Delivered

- [x] `GET /api/v1/plans` — list cluster migration plans
- [x] `GET /api/v1/plans/{namespace}/{name}` — plan + translations in namespace
- [x] `GET /api/v1/plans/{namespace}/{name}/export` — multi-doc YAML (plan, translations, rollout preview)
- [x] Portal Migration Plans tab: wave review, translation preview, download export
- [x] `ambientor-plan::build_export_yaml`

## Test plan

- [x] `cargo test -p ambientor-plan`
- [x] `cargo test --workspace`
- [ ] Lab: open portal Plans tab after inventory scan; download export YAML
