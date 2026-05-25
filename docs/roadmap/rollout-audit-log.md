# Rollout audit log (Phase 3.4)

## Goal

Persist approve / apply / rollback actions to Postgres `audit_events` and expose them via API and portal.

## Delivered

- [x] Map operator `RolloutEvent` → `audit_events` (`rollout.stage.apply`, `rollout.rollback`, etc.)
- [x] `POST .../rollouts/.../approve` → `rollout.approve` audit row (`actor` from body, default `api`)
- [x] `GET /api/v1/audit` — recent events (`?resource=&limit=`)
- [x] `GET /api/v1/rollouts/{ns}/{name}/audit` — per-rollout log
- [x] Portal rollout detail audit list
- [x] Operator writes audits when `DATABASE_URL` is set

## Branch

`feature/rollout-audit-log`
