# Helm production hardening (Tier 4)

## Goal

Wire secrets, OIDC, health probes, and external access in the Ambientor chart so pilots do not hand-edit Deployments.

## Delivered

- [x] `templates/secret.yaml` — `jwt-secret` (+ optional `oidc-client-secret`) with `helm.sh/resource-policy: keep`
- [x] `auth.*` values — `createSecret`, `existingSecret`, `jwtSecret`, `auth.oidc.*`
- [x] API Deployment — OIDC env from values/secret; `DATABASE_URL` from inline URL, bundled Postgres, or `database.existingSecret`
- [x] Liveness/readiness — API `/healthz` + `/readyz`; web `/` (disable via `probes.*.enabled`)
- [x] Optional Ingress (separate API + web hosts) and OpenShift `Route` templates
- [x] `web.apiUrl` / `openshift.apiUrl` for portal `config.js` when API is external

## Install examples

**Lab (kind):**

```bash
helm upgrade --install ambientor deploy/helm/ambientor/ \
  -n ambientor-system --create-namespace \
  -f deploy/helm/ambientor/values-lab.yaml
```

**Production pilot with OIDC:**

```bash
helm upgrade --install ambientor deploy/helm/ambientor/ \
  -n ambientor-system --create-namespace \
  --set auth.createSecret=true \
  --set auth.jwtSecret="$(openssl rand -base64 32)" \
  --set auth.oidc.enabled=true \
  --set auth.oidc.issuerUrl=https://idp.example.com/realms/ambientor \
  --set auth.oidc.clientId=ambientor \
  --set auth.oidc.redirectUri=https://api.example.com/api/v1/auth/oidc/callback \
  --set auth.oidc.clientSecret="$OIDC_CLIENT_SECRET" \
  --set auth.oidc.successUrl=https://portal.example.com/ \
  --set openshift.apiUrl=https://api.example.com \
  --set postgresql.auth.password="$(openssl rand -base64 16)"
```

**External Postgres:**

```bash
--set postgresql.enabled=false \
--set database.existingSecret.name=ambientor-db \
--set database.existingSecret.key=database-url
```

## Notes

- OIDC still uses in-memory PKCE state (single API replica); externalize for HA.
- Change default `postgresql.auth.password` before any production install.
- E2E disables secrets and probes: `values-e2e.yaml`.

## OpenShift pilot

For real-cluster testing use `values-openshift-pilot.yaml` and:

```bash
export AMBIENTOR_STORAGE_CLASS=<your-sc>
./scripts/openshift-pilot-install.sh
./scripts/openshift-pilot-smoke.sh
```

See [runbook-openshift-pilot.md](../runbook-openshift-pilot.md). Routes omit `host` when empty so OpenShift assigns `*.apps.<cluster>`.

## Branch

`main` (Tier 4)
