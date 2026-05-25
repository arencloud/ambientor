# MigrationPlan controller (Phase 2, Step 2.1)

Branch: `feature/migration-plan-controller`

## Goal

When an `AmbientAssessment` completes, materialize a `MigrationPlan` CR with waves from `ambientor-plan::build_plan`.

## Delivered

- [x] Operator `MigrationPlan` controller — reads `spec.assessmentRef`, waits for Completed assessment, patches waves + status
- [x] Assessment controller ensures `{assessment}-plan` CR after scan completes
- [x] `ambientor-plan` helpers: `plan_name_for_assessment`, `namespaces_from_findings`, `assessment_result_from_status`

## Flow

```text
MeshInventory (triggerScan) → AmbientAssessment (Completed) → MigrationPlan (Ready)
```

## Test plan

- [x] `cargo test -p ambientor-plan`
- [x] `cargo test --workspace`
- [ ] Lab: trigger inventory scan; verify `kubectl get migrationplans` shows Ready with waves
