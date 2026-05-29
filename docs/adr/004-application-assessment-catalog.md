# ADR 004: Application assessment catalog (DB + portal)

## Status

Accepted

## Context

- Fleets may have thousands of namespaces across multiple clusters and istiod revisions.
- The portal needs an application-first list (not CRD sidebar) with readiness %, risk, mesh, ingress layout, and drill-down findings.
- Dashboard migration status should reuse the same assessment run.

## Decision

1. **Tables** (`004_application_assessments.sql`): `assessment_runs` + `application_assessments` (one row per namespace per run).
2. **`ambientor-dashboard`**: `build_cluster_assessment_from_context` derives per-namespace rows from rule findings, mesh discovery, VS/HTTPRoute hostnames, ingress gateway pods, and namespace labels.
3. **Run assessment** (`POST /api/v1/assess`): evaluates rules, replaces the latest run in Postgres, then writes **the same run** to `dashboard_snapshots` (not a separate live-only pass).
4. **Portal API**: paginated `GET /api/v1/applications`, detail `GET /api/v1/applications/{namespace}` (requires `DATABASE_URL`).
5. **UI**: sortable table, filters, server pagination (50/page), detail drawer with suggestions + findings.

## Risk and readiness

- **Readiness %** = per-namespace overall score from findings (0–100).
- **Risk** = `critical` (blockers), `high` (readiness &lt; 50), `medium` (warnings or readiness &lt; 80), else `low`.

## Consequences

- Without Postgres, applications API returns 503; run assessment still returns cluster-level findings but does not populate the catalog.
- Hub multicluster: each `cluster_ref` has its own latest run; filter via `?clusterRef=` on applications API.
