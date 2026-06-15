# Full OIDC (Phase 4.1)

## Goal

Replace the manual authorize-URL placeholder with OpenID Connect discovery, PKCE, and an authorization-code callback that issues Ambientor JWTs.

## Delivered

- [x] `OidcFlowService` — discovery, PKCE, CSRF state store, code exchange, ID token claims
- [x] `AuthService::login_oidc` — `find_or_create_oidc` + JWT
- [x] `GET /api/v1/auth/oidc/login` — redirect to IdP
- [x] `GET /api/v1/auth/oidc/callback` — exchange code; JSON token or redirect with `?token=`
- [x] Env: `AMBIENTOR_OIDC_ISSUER_URL`, `AMBIENTOR_OIDC_CLIENT_ID`, `AMBIENTOR_OIDC_REDIRECT_URI`, `AMBIENTOR_OIDC_CLIENT_SECRET`, optional `AMBIENTOR_OIDC_SCOPES`, `AMBIENTOR_OIDC_DEFAULT_ROLES`, `AMBIENTOR_OIDC_SUCCESS_URL`

## Branch

Merged via PR [#20](https://github.com/arencloud/ambientor/pull/20).

## Notes

- In-memory CSRF/PKCE map (single API replica); use sticky sessions or external store for HA.
- OIDC requires Postgres (`DATABASE_URL`) for user provisioning.
- Helm: see [helm-production.md](helm-production.md) for `auth.oidc.*` values and `ambientor-secrets`.
