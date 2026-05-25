# Policy translation (Phase 2, Step 2.2)

Branch: `feature/policy-translation`

## Goal

Suggest Gateway API `HTTPRoute` manifests from Istio `VirtualService` resources for migration planning.

## Delivered

- [x] `ambientor-analyze::virtual_service_to_httproute` — prefix/exact URI, hosts, backendRefs
- [x] Operator `PolicyTranslation` controller — reads VS, writes `status.suggestedManifest` (YAML)
- [x] MigrationPlan reconcile ensures translation CRs per wave namespace

## Test plan

- [x] Unit tests in `ambientor-analyze/src/translate.rs`
- [x] `cargo test --workspace`
- [ ] Lab: after inventory scan, `kubectl get policytranslations -A` and inspect suggested YAML
