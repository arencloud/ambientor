# Performance — informer cache (Phase 5.3)

## Goal

Avoid repeated cluster-wide Pod/Namespace `list` calls during assessments on large clusters (~10k pods) by maintaining kube-runtime reflector caches in the operator.

## Delivered

- [x] `ClusterResourceCache` — pod + namespace reflectors (`ambientor-k8s`)
- [x] `collect_inventory(..., core: Option<CoreSnapshot>)` — uses cache snapshot when populated
- [x] `scan_platform` reuses pod slice (no second pod list when cache provided)
- [x] Operator assessment wired to cache; CLI/API still list on demand
- [x] Unit test: 10k pod aggregation under 500ms

## Branch

Merged via PR [#26](https://github.com/arencloud/ambientor/pull/26).

## Behavior

- Operator starts reflectors at boot; assessments read from in-memory store when populated.
- Before the first watch sync, assessments fall back to API list (same as before).
- Memory scales with cached objects; see [kube-runtime reflector docs](https://docs.rs/kube/latest/kube/runtime/reflector/index.html) for tuning (e.g. `PartialObjectMeta` later).

## Metrics

Log fields: `cache.pod_count` / `cache.namespace_count` can be added in a follow-up; today use operator logs after sync.
