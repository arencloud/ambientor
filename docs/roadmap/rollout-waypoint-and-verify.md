# Rollout: waypoint, policy, restart, verify (Phase 3.1)

## Goal

Execute real Kubernetes changes for migration rollout stages instead of stub messages.

## Delivered

- [x] `DeployWaypoint` — SSA `Gateway` (`istio-waypoint`, HBONE listener) + `istio.io/use-waypoint=waypoint` on namespace
- [x] `TranslatePolicy` — VS → HTTPRoute via `virtual_service_to_httproute`, apply manifest, upsert `PolicyTranslation` status
- [x] `RollingRestart` — SSA pod-template `ambientor.io/restartedAt` on all Deployments in stage namespaces
- [x] `VerifyTraffic` — ambient + use-waypoint labels, waypoint Gateway programmed, VS covered by translated HTTPRoutes
- [x] `plan_to_rollout` includes `{wave}-translate` stage after waypoint

## Notes

- Verify may return “not programmed yet” until Istio programs the waypoint; operator requeues rollouts every 10s.
- Rollback (3.2) still decrements stage index only; does not revert labels/manifests yet.

## Branch

`feature/rollout-waypoint-and-verify`
