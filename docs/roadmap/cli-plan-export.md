# CLI plan create + GitOps export (Phase 2, Step 2.4)

Branch: `feature/cli-plan-export`

## Goal

CLI workflows for generating and exporting migration plans without the portal.

## Delivered

- [x] `ambientor plan create` — assess cluster, build waves, emit YAML bundle or `--json`
- [x] `ambientor plan create --out DIR` — `migration-bundle.yaml` + `plan.json`
- [x] `ambientor plan export -n NS --name PLAN` — cluster or `AMBIENTOR_API_URL` export
- [x] `ambientor-plan::migration_plan_cr` helper for local export

## Usage

```bash
ambientor plan create --out ./gitops
ambientor plan create --json
ambientor plan export -n default --name lab-assessment-plan -o bundle.yaml
AMBIENTOR_API_URL=http://... ambientor plan export -n default --name lab-assessment-plan
```

## Test plan

- [x] `cargo test --workspace`
- [ ] Lab: compare CLI export with portal download
