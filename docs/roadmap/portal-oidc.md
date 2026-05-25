# Portal OIDC and approve gates (production pilot P4)

## Goal

When the API runs with Postgres auth (`DATABASE_URL`), rollout approve must send a JWT; the portal collects tokens via local login or OIDC callback and attaches `Authorization: Bearer`.

## Delivered

- [x] `GET /api/v1/auth/config` — `enabled`, `localLogin`, `oidcLoginUrl`, `requireAuthForApprove`
- [x] Portal header: local sign-in, SSO link, sign-out, session in `localStorage`
- [x] OIDC return: parse `?token=` on load (from `AMBIENTOR_OIDC_SUCCESS_URL` redirect)
- [x] `POST .../rollouts/.../approve` uses Bearer; API actor from JWT claims when token present
- [x] Approve button disabled + hint when auth required but not signed in
- [x] Rollouts panel: fixed duplicate **Stages** heading; audit below stage table

## Configuration

| Variable | Purpose |
|----------|---------|
| `DATABASE_URL` | Enables auth + Casbin |
| `AMBIENTOR_JWT_SECRET` | JWT signing |
| `AMBIENTOR_OIDC_*` | IdP login (see [oidc-auth.md](oidc-auth.md)) |
| `AMBIENTOR_OIDC_SUCCESS_URL` | Portal URL; callback appends `?token=` |

Portal uses `window.AMBIENTOR_API_URL` from `/config.js` (same origin or API gateway). OIDC login navigates to `{API}/api/v1/auth/oidc/login`.

## Branch

`feature/portal-oidc-pilot`

## Pilot validation

See [runbook-pilot.md](../runbook-pilot.md) § P4.
