# Rollout rollback reverts cluster state (Phase 3.2)

## Goal

On stage failure with `autoRollback: true`, undo Kubernetes changes from completed stages instead of only decrementing `currentStage`.

## Delivered

- [x] Revert completed stages in reverse order (`0..failed_at`)
- [x] `LabelNamespace` — remove `istio.io/dataplane-mode`
- [x] `DeployWaypoint` — delete Ambientor-managed `waypoint` Gateway; remove `istio.io/use-waypoint`
- [x] `TranslatePolicy` — delete HTTPRoutes with `ambientor.io/translated-from` and matching `PolicyTranslation` CRs
- [x] `RollingRestart` — documented no-op (pods not rolled back)
- [x] `EnrollNamespace` — remove enrollment labels; drop OSSM `ServiceMeshMemberRoll` member when applicable
- [x] `RemoveInjection` — restore `istio-injection: enabled` on namespace
- [x] `InstallAmbientComponents` — no-op (preflight only)
- [x] Reset `currentStage` and `approvedStage` to 0; clear `stageResults`; phase `RolledBack`
- [x] kind e2e: `scripts/e2e-kind-ambient.sh` injects verify failure → `RolledBack` before happy-path rollout

## Branch

`feature/rollout-rollback-revert`
