# Mesh inventory + Istio CR parsing

Branch: `feature/mesh-inventory-istio-crds`

## Goal

Populate `RuleContext.policies` and `mesh_version` from live cluster state so readiness, traffic, and planner rules reflect real Istio/OSSM configuration.

## Tasks

### Done in this branch

- [x] Dynamic list helpers for Istio + Gateway API CRDs
- [x] `PolicyContext` population (PeerAuthentication, AuthZ, VS, HTTPRoute, EnvoyFilter, WasmPlugin)
- [x] Istio control-plane version detection (istiod deployment labels)
- [x] Unit tests with JSON fixtures
- [x] Wire into `collect_inventory`

### Next (follow-up branches)

Track all phases: [PROGRESS.md](../PROGRESS.md). Lab runbook: [runbook-lab.md](../runbook-lab.md).

- [ ] Namespace-scoped list (discoverySelectors / member roll) for OSSM
- [ ] DestinationRule + sidecar workload spec scan (`ambientor-scan`)
- [ ] Evidence attachments on findings (YAML snippets in CR status)
- [ ] Replace operator polling with informer-based controllers
- [ ] Integration test against kind with Istio ambient profile

## CRDs collected

| GVK | PolicyContext field |
|-----|---------------------|
| `security.istio.io/v1/PeerAuthentication` | `peer_auth_disable` |
| `security.istio.io/v1/AuthorizationPolicy` | `l7_authorization_policies` |
| `networking.istio.io/v1/VirtualService` | `virtual_services` |
| `gateway.networking.k8s.io/v1/HTTPRoute` | `http_routes` |
| `networking.istio.io/v1/EnvoyFilter` | `envoy_filters` |
| `extensions.istio.io/v1/WasmPlugin` | (future) |
