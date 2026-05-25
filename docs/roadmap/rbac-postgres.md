# Namespace-scoped Casbin in Postgres (Phase 4.2)

## Goal

Persist RBAC policies in Postgres with namespace domains, and gate rollout approval on JWT + Casbin when auth is enabled.

## Delivered

- [x] `casbin_rule` table migration (`002_casbin_rule.sql`) + `sqlx-adapter` on shared `PgPool`
- [x] Domain Casbin model (`sub, dom, obj, act`) with `keyMatch` on namespace and object
- [x] `RbacEnforcer::with_postgres` — load policies, seed defaults when empty, save to DB
- [x] `AuthService::authorize(claims, namespace, object, action)`
- [x] `POST .../rollouts/{namespace}/{name}/approve` requires `Authorization: Bearer` when `DATABASE_URL` is set
- [x] `assign_role(user, role, namespace)` persists grouping policies

## Branch

Merged via PR [#21](https://github.com/arencloud/ambientor/pull/21).

## Notes

- Global roles use policy domain `*` (matches any namespace via `keyMatch`).
- Per-namespace role bindings: `g, user, role, namespace` via `assign_role`.
- Legacy `rbac_policies` table from `001_init.sql` is unused; `casbin_rule` is canonical.
