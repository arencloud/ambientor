# Rollout rollback reverts cluster state (Phase 3.2)

## Goal

On stage failure with `autoRollback: true`, undo Kubernetes changes from completed stages instead of only decrementing `currentStage`.

## Delivered

- [x] Revert completed stages in reverse order (`0..failed_at`)
- [x] `LabelNamespace` — remove `istio.io/dataplane-mode`
- [x] `DeployWaypoint` — delete Ambientor-managed `waypoint` Gateway; remove `istio.io/use-waypoint`
- [x] `TranslatePolicy` — delete HTTPRoutes with `ambientor.io/translated-from` and matching `PolicyTranslation` CRs
- [x] `RollingRestart` — documented no-op (pods not rolled back)
- [x] Reset `currentStage` and `approvedStage` to 0; clear `stageResults`; phase `RolledBack`

## Branch

`cursor/rollout-rollback-revert`
