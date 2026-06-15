use std::collections::HashMap;

use ambientor_dashboard::{
    ApplicationAssessmentRecord, ClusterAssessmentRun, DashboardResponse,
    FleetClusterDashboard, FleetDashboardResponse, MeshInstanceDashboard, RiskLevel, StatusCounts,
    dashboard_from_assessment_run, derive_dataplane_mode_from_stored,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::pool::DbError;
use crate::traits::DashboardStore;

pub struct DashboardRepository {
    pool: PgPool,
}

impl DashboardRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn sync_snapshot(&self, response: &DashboardResponse) -> Result<(), DbError> {
        let mut tx = self.pool.begin().await?;
        let cluster_id = upsert_cluster(&mut tx, response).await?;
        let mesh_ids = upsert_mesh_instances(&mut tx, cluster_id, &response.mesh_instances).await?;
        sync_application_status(&mut tx, cluster_id, &response.mesh_instances, &mesh_ids).await?;

        let summary_json = serde_json::to_value(&response.summary)
            .map_err(|e| DbError::Serialize(e.to_string()))?;
        let mesh_json = serde_json::to_value(&response.mesh_instances)
            .map_err(|e| DbError::Serialize(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT INTO dashboard_snapshots (id, cluster_id, summary, mesh_instances, captured_at)
            VALUES ($1, $2, $3, $4, NOW())
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(cluster_id)
        .bind(summary_json)
        .bind(mesh_json)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(())
    }

    pub async fn load_by_cluster_ref(
        &self,
        cluster_ref: &str,
    ) -> Result<Option<DashboardResponse>, DbError> {
        let row = sqlx::query_as::<_, SnapshotRow>(
            r#"
            SELECT
                c.cluster_ref,
                c.display_name,
                c.platform,
                c.mesh_flavor,
                c.istio_version,
                s.summary,
                s.mesh_instances,
                s.captured_at
            FROM clusters c
            INNER JOIN LATERAL (
                SELECT summary, mesh_instances, captured_at
                FROM dashboard_snapshots
                WHERE cluster_id = c.id
                ORDER BY captured_at DESC
                LIMIT 1
            ) s ON true
            WHERE c.cluster_ref = $1
            "#,
        )
        .bind(cluster_ref)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| r.into_response()).transpose()
    }

    /// True when a newer assessment run exists than the latest dashboard snapshot.
    pub async fn is_snapshot_stale(&self, cluster_ref: &str) -> Result<bool, DbError> {
        let row: (Option<DateTime<Utc>>, Option<DateTime<Utc>>) = sqlx::query_as(
            r#"
            SELECT
                (SELECT MAX(ar.finished_at)
                 FROM assessment_runs ar
                 INNER JOIN clusters c ON c.id = ar.cluster_id
                 WHERE c.cluster_ref = $1),
                (SELECT MAX(ds.captured_at)
                 FROM dashboard_snapshots ds
                 INNER JOIN clusters c ON c.id = ds.cluster_id
                 WHERE c.cluster_ref = $1)
            "#,
        )
        .bind(cluster_ref)
        .fetch_one(&self.pool)
        .await?;

        match (row.0, row.1) {
            (Some(assess_at), Some(dash_at)) => Ok(assess_at > dash_at),
            (Some(_), None) => Ok(true),
            _ => Ok(false),
        }
    }

    /// Rebuild dashboard view from the latest `application_assessments` run (no live API calls).
    pub async fn rebuild_from_latest_assessment(
        &self,
        cluster_ref: &str,
    ) -> Result<Option<DashboardResponse>, DbError> {
        let cluster_row: Option<ClusterMetaRow> = sqlx::query_as(
            r#"
            SELECT display_name, platform, mesh_flavor, istio_version
            FROM clusters WHERE cluster_ref = $1
            "#,
        )
        .bind(cluster_ref)
        .fetch_optional(&self.pool)
        .await?;

        let Some(cluster_row) = cluster_row else {
            return Ok(None);
        };

        let apps = sqlx::query_as::<_, AppAssessmentRow>(
            r#"
            SELECT
                aa.namespace, aa.application_name, aa.workload_components, aa.migration_candidate,
                aa.mesh_revision, aa.discovery_label, aa.control_plane_namespace,
                aa.hostnames, aa.namespace_labels, aa.dataplane_mode, aa.ingress_gateway_namespace,
                aa.ingress_same_namespace, aa.workload_count, aa.readiness_pct, aa.risk_level,
                aa.blocker_count, aa.warning_count, aa.scores, aa.summary, aa.findings, aa.suggestions
            FROM application_assessments aa
            INNER JOIN clusters c ON c.id = aa.cluster_id
            INNER JOIN assessment_runs ar ON ar.id = aa.run_id
            WHERE c.cluster_ref = $1
              AND ar.id = (
                SELECT id FROM assessment_runs
                WHERE cluster_id = c.id
                ORDER BY finished_at DESC
                LIMIT 1
              )
            "#,
        )
        .bind(cluster_ref)
        .fetch_all(&self.pool)
        .await?;

        if apps.is_empty() {
            return Ok(None);
        }

        let applications: Result<Vec<ApplicationAssessmentRecord>, DbError> =
            apps.into_iter().map(|r| r.into_record()).collect();
        let applications = applications?;

        let run = ClusterAssessmentRun {
            cluster_ref: cluster_ref.to_string(),
            applications,
            cluster_scores: Default::default(),
            cluster_summary: Default::default(),
            cluster_findings: Vec::new(),
        };

        let cluster = ambientor_dashboard::ClusterDashboard {
            name: cluster_row.display_name,
            platform: cluster_row.platform.unwrap_or_else(|| "Kubernetes".into()),
            mesh_flavor: cluster_row.mesh_flavor.unwrap_or_default(),
            istio_version: cluster_row.istio_version,
            mesh_instance_count: 0,
            ambient_mesh_count: 0,
        };

        let response = dashboard_from_assessment_run(&run, cluster);
        Ok(Some(response))
    }

    pub async fn load_fleet(&self) -> Result<Option<FleetDashboardResponse>, DbError> {
        let rows = sqlx::query_as::<_, SnapshotRow>(
            r#"
            SELECT
                c.cluster_ref,
                c.display_name,
                c.platform,
                c.mesh_flavor,
                c.istio_version,
                s.summary,
                s.mesh_instances,
                s.captured_at
            FROM clusters c
            INNER JOIN LATERAL (
                SELECT summary, mesh_instances, captured_at
                FROM dashboard_snapshots
                WHERE cluster_id = c.id
                ORDER BY captured_at DESC
                LIMIT 1
            ) s ON true
            ORDER BY c.cluster_ref
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        if rows.is_empty() {
            return Ok(None);
        }

        let mut clusters = Vec::with_capacity(rows.len());
        let mut summaries = Vec::with_capacity(rows.len());
        let mut last_updated = None::<DateTime<Utc>>;

        for row in rows {
            let dash = row.into_response()?;
            if let Ok(ts) = DateTime::parse_from_rfc3339(&dash.last_updated) {
                let ts = ts.with_timezone(&Utc);
                last_updated = Some(last_updated.map_or(ts, |prev| prev.max(ts)));
            }
            summaries.push(dash.summary.clone());
            clusters.push(FleetClusterDashboard {
                cluster_ref: dash.cluster_ref,
                cluster: dash.cluster,
                summary: dash.summary,
                mesh_instances: dash.mesh_instances,
                last_updated: dash.last_updated,
            });
        }

        let summary = ambientor_dashboard::aggregate_fleet_summary(&summaries);
        Ok(Some(FleetDashboardResponse {
            summary,
            clusters,
            last_updated: last_updated
                .map(|t| t.to_rfc3339())
                .unwrap_or_else(|| Utc::now().to_rfc3339()),
        }))
    }
}

#[derive(sqlx::FromRow)]
struct SnapshotRow {
    cluster_ref: String,
    display_name: String,
    platform: Option<String>,
    mesh_flavor: Option<String>,
    istio_version: Option<String>,
    summary: Value,
    mesh_instances: Value,
    captured_at: DateTime<Utc>,
}

impl SnapshotRow {
    fn into_response(self) -> Result<DashboardResponse, DbError> {
        let summary: StatusCounts = serde_json::from_value(self.summary)
            .map_err(|e| DbError::Serialize(e.to_string()))?;
        let mesh_instances: Vec<MeshInstanceDashboard> = serde_json::from_value(self.mesh_instances)
            .map_err(|e| DbError::Serialize(e.to_string()))?;
        let mesh_instance_count = mesh_instances.len();
        let ambient_mesh_count = mesh_instances.iter().filter(|m| m.ambient).count();
        let migration_savings = Some(ambientor_dashboard::compute_migration_savings_from_dashboard(
            &mesh_instances,
        ));

        Ok(DashboardResponse {
            cluster_ref: self.cluster_ref,
            cluster: ambientor_dashboard::ClusterDashboard {
                name: self.display_name,
                platform: self.platform.unwrap_or_else(|| "Kubernetes".into()),
                mesh_flavor: self.mesh_flavor.unwrap_or_default(),
                istio_version: self.istio_version,
                mesh_instance_count,
                ambient_mesh_count,
            },
            summary,
            mesh_instances,
            migration_savings,
            last_updated: self.captured_at.to_rfc3339(),
            connection_namespace: None,
            connection_name: None,
            reachable: None,
            is_hub: None,
        })
    }
}

fn connection_parts_from_response(response: &DashboardResponse) -> (Option<String>, Option<String>) {
    if let (Some(ns), Some(name)) = (
        &response.connection_namespace,
        &response.connection_name,
    ) {
        return (Some(ns.clone()), Some(name.clone()));
    }
    if response.cluster_ref == "in-cluster" || !response.cluster_ref.contains('/') {
        return (None, None);
    }
    if let Some((ns, name)) = response.cluster_ref.split_once('/') {
        if !ns.is_empty() && !name.is_empty() {
            return (Some(ns.into()), Some(name.into()));
        }
    }
    (None, None)
}

pub(crate) async fn upsert_cluster(
    tx: &mut Transaction<'_, Postgres>,
    response: &DashboardResponse,
) -> Result<Uuid, DbError> {
    let id = Uuid::new_v4();
    let (connection_namespace, connection_name) = connection_parts_from_response(response);
    let is_hub = response.is_hub.unwrap_or(connection_namespace.is_none());
    let reachable = response.reachable.unwrap_or(true);
    let row: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO clusters (
            id, cluster_ref, display_name, platform, mesh_flavor, istio_version,
            is_hub, connection_namespace, connection_name, reachable,
            last_seen_at, updated_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, NOW(), NOW())
        ON CONFLICT (cluster_ref) DO UPDATE SET
            display_name = EXCLUDED.display_name,
            platform = EXCLUDED.platform,
            mesh_flavor = EXCLUDED.mesh_flavor,
            istio_version = EXCLUDED.istio_version,
            is_hub = EXCLUDED.is_hub,
            connection_namespace = EXCLUDED.connection_namespace,
            connection_name = EXCLUDED.connection_name,
            reachable = EXCLUDED.reachable,
            last_seen_at = NOW(),
            updated_at = NOW()
        RETURNING id
        "#,
    )
    .bind(id)
    .bind(&response.cluster_ref)
    .bind(&response.cluster.name)
    .bind(&response.cluster.platform)
    .bind(&response.cluster.mesh_flavor)
    .bind(&response.cluster.istio_version)
    .bind(is_hub)
    .bind(connection_namespace)
    .bind(connection_name)
    .bind(reachable)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.0)
}

async fn upsert_mesh_instances(
    tx: &mut Transaction<'_, Postgres>,
    cluster_id: Uuid,
    meshes: &[MeshInstanceDashboard],
) -> Result<HashMap<(String, String, String), Uuid>, DbError> {
    let mut map = HashMap::new();
    for mesh in meshes {
        let id = Uuid::new_v4();
        let row: (Uuid,) = sqlx::query_as(
            r#"
            INSERT INTO mesh_instances (
                id, cluster_id, revision, discovery_label, control_plane_namespace,
                version, ambient, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, NOW())
            ON CONFLICT (cluster_id, revision, discovery_label, control_plane_namespace)
            DO UPDATE SET
                version = EXCLUDED.version,
                ambient = EXCLUDED.ambient,
                updated_at = NOW()
            RETURNING id
            "#,
        )
        .bind(id)
        .bind(cluster_id)
        .bind(&mesh.revision)
        .bind(&mesh.discovery_label)
        .bind(&mesh.control_plane_namespace)
        .bind(&mesh.version)
        .bind(mesh.ambient)
        .fetch_one(&mut **tx)
        .await?;
        map.insert(
            (
                mesh.revision.clone(),
                mesh.discovery_label.clone(),
                mesh.control_plane_namespace.clone(),
            ),
            row.0,
        );
    }
    Ok(map)
}

async fn sync_application_status(
    tx: &mut Transaction<'_, Postgres>,
    cluster_id: Uuid,
    meshes: &[MeshInstanceDashboard],
    mesh_ids: &HashMap<(String, String, String), Uuid>,
) -> Result<(), DbError> {
    let mut namespaces: Vec<String> = Vec::new();
    for mesh in meshes {
        for app in &mesh.applications {
            namespaces.push(app.namespace.clone());
        }
    }

    if namespaces.is_empty() {
        sqlx::query("DELETE FROM application_status WHERE cluster_id = $1")
            .bind(cluster_id)
            .execute(&mut **tx)
            .await?;
        return Ok(());
    }

    sqlx::query(
        "DELETE FROM application_status WHERE cluster_id = $1 AND NOT (namespace = ANY($2))",
    )
    .bind(cluster_id)
    .bind(&namespaces)
    .execute(&mut **tx)
    .await?;

    for mesh in meshes {
        let mesh_key = (
            mesh.revision.clone(),
            mesh.discovery_label.clone(),
            mesh.control_plane_namespace.clone(),
        );
        let mesh_instance_id = mesh_ids.get(&mesh_key).copied();
        for app in &mesh.applications {
            let status = app.status.as_db_str();
            sqlx::query(
                r#"
                INSERT INTO application_status (
                    id, cluster_id, mesh_instance_id, namespace, status,
                    ambient_dataplane, blocker_count, rollout_phase, assessment_ref,
                    mesh_revision, discovery_label, synced_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, NOW())
                ON CONFLICT (cluster_id, namespace) DO UPDATE SET
                    mesh_instance_id = EXCLUDED.mesh_instance_id,
                    status = EXCLUDED.status,
                    ambient_dataplane = EXCLUDED.ambient_dataplane,
                    blocker_count = EXCLUDED.blocker_count,
                    rollout_phase = EXCLUDED.rollout_phase,
                    assessment_ref = EXCLUDED.assessment_ref,
                    mesh_revision = EXCLUDED.mesh_revision,
                    discovery_label = EXCLUDED.discovery_label,
                    synced_at = NOW()
                "#,
            )
            .bind(Uuid::new_v4())
            .bind(cluster_id)
            .bind(mesh_instance_id)
            .bind(&app.namespace)
            .bind(status)
            .bind(app.ambient_dataplane)
            .bind(app.blocker_count as i32)
            .bind(&app.rollout_phase)
            .bind(&app.assessment_ref)
            .bind(&app.mesh_revision)
            .bind(&app.discovery_label)
            .execute(&mut **tx)
            .await?;
        }
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct ClusterMetaRow {
    display_name: String,
    platform: Option<String>,
    mesh_flavor: Option<String>,
    istio_version: Option<String>,
}

#[derive(sqlx::FromRow)]
struct AppAssessmentRow {
    namespace: String,
    application_name: String,
    workload_components: serde_json::Value,
    migration_candidate: bool,
    mesh_revision: Option<String>,
    discovery_label: Option<String>,
    control_plane_namespace: Option<String>,
    hostnames: serde_json::Value,
    namespace_labels: serde_json::Value,
    dataplane_mode: String,
    ingress_gateway_namespace: Option<String>,
    ingress_same_namespace: bool,
    workload_count: i32,
    readiness_pct: i16,
    risk_level: String,
    blocker_count: i32,
    warning_count: i32,
    scores: serde_json::Value,
    summary: serde_json::Value,
    findings: serde_json::Value,
    suggestions: serde_json::Value,
}

impl AppAssessmentRow {
    fn into_record(self) -> Result<ApplicationAssessmentRecord, DbError> {
        let namespace_labels: std::collections::BTreeMap<String, String> =
            serde_json::from_value(self.namespace_labels)
                .map_err(|e| DbError::Serialize(e.to_string()))?;
        let dataplane_mode = if self.dataplane_mode == "ambient" || self.dataplane_mode == "sidecar" {
            self.dataplane_mode
        } else {
            let derived = derive_dataplane_mode_from_stored(
                &namespace_labels,
                self.mesh_revision.as_deref(),
                self.discovery_label.as_deref(),
            );
            if derived != ambientor_dashboard::DataplaneMode::NotEnrolled {
                derived.as_str().to_string()
            } else {
                self.dataplane_mode
            }
        };
        let workload_components: Vec<String> =
            serde_json::from_value(self.workload_components)
                .unwrap_or_default();
        let application_name = if self.application_name.is_empty() {
            self.namespace.clone()
        } else {
            self.application_name
        };
        let dp_mode = match dataplane_mode.as_str() {
            "ambient" => ambientor_dashboard::DataplaneMode::Ambient,
            "sidecar" => ambientor_dashboard::DataplaneMode::Sidecar,
            _ => derive_dataplane_mode_from_stored(
                &namespace_labels,
                self.mesh_revision.as_deref(),
                self.discovery_label.as_deref(),
            ),
        };
        let migration_candidate = self.migration_candidate
            && dp_mode != ambientor_dashboard::DataplaneMode::Ambient;

        Ok(ApplicationAssessmentRecord {
            namespace: self.namespace,
            application_name,
            workload_components,
            migration_candidate,
            mesh_revision: self.mesh_revision,
            discovery_label: self.discovery_label,
            control_plane_namespace: self.control_plane_namespace,
            hostnames: serde_json::from_value(self.hostnames)
                .map_err(|e| DbError::Serialize(e.to_string()))?,
            namespace_labels,
            dataplane_mode,
            ingress_gateway_namespace: self.ingress_gateway_namespace,
            ingress_same_namespace: self.ingress_same_namespace,
            workload_count: self.workload_count as u32,
            readiness_pct: self.readiness_pct as u8,
            risk_level: RiskLevel::from_db_str(&self.risk_level),
            blocker_count: self.blocker_count as u32,
            warning_count: self.warning_count as u32,
            scores: serde_json::from_value(self.scores)
                .map_err(|e| DbError::Serialize(e.to_string()))?,
            summary: serde_json::from_value(self.summary)
                .map_err(|e| DbError::Serialize(e.to_string()))?,
            findings: serde_json::from_value(self.findings)
                .map_err(|e| DbError::Serialize(e.to_string()))?,
            suggestions: serde_json::from_value(self.suggestions)
                .map_err(|e| DbError::Serialize(e.to_string()))?,
        })
    }
}

#[async_trait]
impl DashboardStore for DashboardRepository {
    async fn sync_snapshot(&self, response: &DashboardResponse) -> Result<(), DbError> {
        DashboardRepository::sync_snapshot(self, response).await
    }

    async fn load_by_cluster_ref(
        &self,
        cluster_ref: &str,
    ) -> Result<Option<DashboardResponse>, DbError> {
        DashboardRepository::load_by_cluster_ref(self, cluster_ref).await
    }

    async fn is_snapshot_stale(&self, cluster_ref: &str) -> Result<bool, DbError> {
        DashboardRepository::is_snapshot_stale(self, cluster_ref).await
    }

    async fn rebuild_from_latest_assessment(
        &self,
        cluster_ref: &str,
    ) -> Result<Option<DashboardResponse>, DbError> {
        DashboardRepository::rebuild_from_latest_assessment(self, cluster_ref).await
    }

    async fn load_fleet(&self) -> Result<Option<FleetDashboardResponse>, DbError> {
        DashboardRepository::load_fleet(self).await
    }
}
