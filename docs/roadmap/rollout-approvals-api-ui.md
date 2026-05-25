# Rollout approvals API and portal UI (Phase 3.3)

## Goal

Let operators approve gated rollout stages via API, CLI, and portal (`approvedStage` on the `Rollout` CR).

## Delivered

- [x] `GET /api/v1/rollouts` — list rollouts with phase and approval state
- [x] `GET /api/v1/rollouts/{namespace}/{name}` — stages + results
- [x] `POST /api/v1/rollouts/{namespace}/{name}/approve` — patch `status.approvedStage`
- [x] `POST /api/v1/plans/{namespace}/{name}/rollout` — create `{plan-name}-rollout` CR
- [x] Portal **Rollouts** panel — list, stage table, approve button
- [x] Portal **Start rollout from plan** on Migration Plans
- [x] CLI `ambientor rollout status|approve` (API or kube)

## Branch

`feature/rollout-approvals-api-ui`
