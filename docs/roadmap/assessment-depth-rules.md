# Assessment depth rules (Phase 1, Step 1.5)

Branch: `cursor/assessment-depth-rules`

## Goal

Add blockers and warnings aligned with Istio ambient migration docs: version gates, SPIRE, EnvoyFilter-on-waypoint, OSSM MemberRoll enrollment.

## Tasks

### Done in this branch

- [x] `PlatformContext` on `RuleContext` (SPIRE hits, OSSM member namespaces)
- [x] `readiness.istio-version` — minimum Istio 1.24
- [x] `readiness.spire-workloads` — SPIRE/SPIFFE detection
- [x] `readiness.envoyfilter-waypoint` — EnvoyFilter on waypoint/gateway
- [x] `platform.ossm-member-roll` — namespaces not in MemberRoll (OSSM flavor)
- [x] OSSM preflight uses live MemberRoll list
- [x] Unit + fixture tests

### Next

- [ ] Step 1.6 namespace-scoped inventory (if not fully covered by MemberRoll rule)
- [ ] Step 1.7 portal assessment UI
