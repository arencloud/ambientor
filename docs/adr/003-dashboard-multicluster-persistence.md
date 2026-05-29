# ADR 003: Dashboard persistence and multicluster schema

## Status

Accepted — schema, operator/API sync, and fleet read API implemented

## Context

- The portal **dashboard** shows per-cluster migration status (Migrated, Processing, Blocker, Failed, Scanned, Not scanned) and per–istiod application rows.
- Today `GET /api/v1/dashboard` **recomputes** everything from the Kubernetes API on each request. Nothing is stored for the dashboard itself.
- `scan_runs` already stores assessment JSON keyed by a loose `cluster_ref` text field (`in-cluster` or `{namespace}/{name}` for `ClusterConnection`).
- Hub mode (`ClusterConnection`) will eventually need a **single portal** listing many OpenShift/Kubernetes clusters without N round-trips to N API servers per page load.

## Decision

### 1. Dashboard UI scope

- **Dashboard** = fleet/cluster migration posture only (summary tiles + istiod cards). No assessment scores, findings drill-down, or “Run assessment”.
- **Assessments** = run/list/view assessments (existing panel owns `POST /api/v1/assess`).

### 2. Persistence model (Postgres)

Introduce normalized tables (migration `003_dashboard_multicluster.sql`):

| Table | Purpose |
|-------|---------|
| `clusters` | Registry: `cluster_ref`, display name, platform, Istio version, hub/spoke link to `ClusterConnection` |
| `mesh_instances` | Control planes per cluster (revision, discovery label, CP namespace, ambient flag) |
| `application_status` | One row per `(cluster, namespace)` — materialized migration status for dashboard queries |
| `dashboard_snapshots` | Optional JSON cache of full dashboard payload per cluster for fast hub reads |

`scan_runs.cluster_id` optionally links historical assessments to `clusters.id` (backfill by matching `cluster_ref`).

### 3. Sync path

1. **`ambientor-dashboard`** — shared `build_dashboard(client, cluster_ref)` from cluster state.
2. **Operator** — 30s sync loop + refresh after `AmbientAssessment` reconcile when `DATABASE_URL` is set.
3. **API** — `GET /api/v1/dashboard` reads latest snapshot from Postgres (fallback: live compute + persist). `POST /api/v1/assess` and remote `connections/.../assess` refresh the cache. `GET /api/v1/dashboard/fleet` returns all clusters with snapshots.
4. **Hub spokes** — remote assess persists a per–`cluster_ref` snapshot; periodic spoke sync from hub is future work.

### 4. `cluster_ref` convention

Stable string, aligned with existing code:

- In-cluster agent: `AMBIENTOR_CLUSTER_REF` or `in-cluster`
- Remote connection: `{connectionNamespace}/{connectionName}` (see `connection_cluster_ref`)

### 5. Status values

Stored as lowercase snake in DB: `migrated`, `processing`, `blocker`, `failed`, `scanned`, `not_scanned` — maps 1:1 to API `ApplicationMigrationStatus`.

## Consequences

- Dashboard can stay fast and multicluster-capable without duplicating assessment blobs (those remain in `scan_runs` / `AmbientAssessment` CRs).
- CRDs remain source of truth; DB is a **materialized view** for portal and hub aggregation, not a second control plane.
- Without `DATABASE_URL`, the API still computes the dashboard live on each request.
- After assessment, dashboard and applications share one run; `GET /api/v1/dashboard?fresh=true` rebuilds from DB when the snapshot is older than the latest `assessment_runs` row.

## Related

- [hub-aggregation.md](../roadmap/hub-aggregation.md)
- [postgres-scan-persistence.md](../roadmap/postgres-scan-persistence.md)
