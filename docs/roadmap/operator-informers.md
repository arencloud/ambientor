# Operator informers (Phase 1, Step 1.4)

Branch: `cursor/operator-informers`

## Goal

Replace 30-second polling loops with kube-runtime watch controllers and avoid duplicate assessment CRs.

## Changes

- `MeshInventory`, `AmbientAssessment`, `Rollout`, `Cluster`, `ClusterConnection` use `Controller::run` + `shutdown_on_signal`.
- Inventory tracks `status.observedGeneration` vs `metadata.generation`; rescans only when spec changes.
- Stable assessment name: `{inventory-name}-assessment` (SSA apply + Pending status reset).
- Hub controller validates credentials Secret exists.

## Test plan

- [x] `cargo test -p ambientor-operator`
- [x] `cargo clippy -p ambientor-operator -- -D warnings`
- [ ] Lab: apply `meshinventory-bookinfo.yaml` twice — single assessment CR, rescans on spec edit
