use ambientor_dashboard::{
    ApplicationAssessmentRecord, ApplicationDetail, ApplicationListItem, ApplicationListPage,
    ClusterAssessmentRun, RiskLevel, derive_dataplane_mode_from_stored,
};
use async_trait::async_trait;
use chrono::Utc;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::dashboard::upsert_cluster;
use crate::pool::DbError;
use crate::traits::ApplicationAssessmentStore;

#[derive(Debug)]
pub struct ApplicationListQuery {
    pub cluster_ref: String,
    pub search: Option<String>,
    pub risk_level: Option<String>,
    pub mesh_revision: Option<String>,
    /// When true (default), only namespaces that still need migration.
    pub migration_candidates_only: bool,
    pub page: u32,
    pub page_size: u32,
}

impl Default for ApplicationListQuery {
    fn default() -> Self {
        Self {
            cluster_ref: String::new(),
            search: None,
            risk_level: None,
            mesh_revision: None,
            migration_candidates_only: true,
            page: 1,
            page_size: 50,
        }
    }
}

pub struct ApplicationAssessmentRepository {
    pool: PgPool,
}

impl ApplicationAssessmentRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn replace_run(&self, run: &ClusterAssessmentRun) -> Result<Uuid, DbError> {
        let mut tx = self.pool.begin().await?;
        let cluster_id = ensure_cluster_for_assessment(&mut tx, &run.cluster_ref).await?;
        let run_id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO assessment_runs (
                id, cluster_id, status, application_count,
                cluster_scores, cluster_summary, cluster_findings, started_at, finished_at
            )
            VALUES ($1, $2, 'completed', $3, $4, $5, $6, NOW(), NOW())
            "#,
        )
        .bind(run_id)
        .bind(cluster_id)
        .bind(run.applications.len() as i32)
        .bind(
            serde_json::to_value(&run.cluster_scores)
                .map_err(|e| DbError::Serialize(e.to_string()))?,
        )
        .bind(
            serde_json::to_value(&run.cluster_summary)
                .map_err(|e| DbError::Serialize(e.to_string()))?,
        )
        .bind(
            serde_json::to_value(&run.cluster_findings)
                .map_err(|e| DbError::Serialize(e.to_string()))?,
        )
        .execute(&mut *tx)
        .await?;

        sqlx::query("DELETE FROM application_assessments WHERE cluster_id = $1")
            .bind(cluster_id)
            .execute(&mut *tx)
            .await?;

        for app in &run.applications {
            insert_application(&mut tx, run_id, cluster_id, app).await?;
        }

        tx.commit().await?;
        Ok(run_id)
    }

    pub async fn list_applications(
        &self,
        query: ApplicationListQuery,
    ) -> Result<ApplicationListPage, DbError> {
        let page = query.page.max(1);
        let page_size = query.page_size.clamp(1, 200);
        let offset = ((page - 1) as i64) * page_size as i64;

        let search = query.search.as_deref().map(|s| format!("%{s}%"));
        let risk = query.risk_level.as_deref();
        let mesh_revision = query.mesh_revision.as_deref();

        let total: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::bigint
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
              AND ($2::text IS NULL OR aa.namespace ILIKE $2 OR aa.application_name ILIKE $2 OR aa.hostnames::text ILIKE $2)
              AND ($3::text IS NULL OR aa.risk_level = $3)
              AND ($4::text IS NULL OR aa.mesh_revision = $4)
              AND ($5::bool IS FALSE OR (aa.migration_candidate = TRUE AND aa.dataplane_mode <> 'ambient'))
            "#,
        )
        .bind(&query.cluster_ref)
        .bind(search.as_deref())
        .bind(risk)
        .bind(mesh_revision)
        .bind(query.migration_candidates_only)
        .fetch_one(&self.pool)
        .await?;

        let excluded_ambient: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)::bigint
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
              AND aa.migration_candidate = FALSE
              AND aa.dataplane_mode = 'ambient'
            "#,
        )
        .bind(&query.cluster_ref)
        .fetch_one(&self.pool)
        .await?;

        let rows = sqlx::query_as::<_, AppRow>(
            r#"
            SELECT
                aa.namespace,
                aa.application_name,
                aa.workload_components,
                aa.migration_candidate,
                c.cluster_ref,
                aa.mesh_revision,
                aa.discovery_label,
                aa.control_plane_namespace,
                aa.hostnames,
                aa.namespace_labels,
                aa.dataplane_mode,
                aa.ingress_gateway_namespace,
                aa.ingress_same_namespace,
                aa.workload_count,
                aa.readiness_pct,
                aa.risk_level,
                aa.blocker_count,
                aa.warning_count,
                ar.id AS run_id,
                ar.finished_at
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
              AND ($2::text IS NULL OR aa.namespace ILIKE $2 OR aa.application_name ILIKE $2 OR aa.hostnames::text ILIKE $2)
              AND ($3::text IS NULL OR aa.risk_level = $3)
              AND ($4::text IS NULL OR aa.mesh_revision = $4)
              AND ($5::bool IS FALSE OR (aa.migration_candidate = TRUE AND aa.dataplane_mode <> 'ambient'))
            ORDER BY aa.application_name ASC, aa.namespace ASC
            LIMIT $6 OFFSET $7
            "#,
        )
        .bind(&query.cluster_ref)
        .bind(search.as_deref())
        .bind(risk)
        .bind(mesh_revision)
        .bind(query.migration_candidates_only)
        .bind(page_size as i64)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        let items: Vec<ApplicationListItem> = rows.iter().map(AppRow::to_list_item).collect();
        let run_id = rows.first().map(|r| r.run_id.to_string());
        let last_assessed_at = rows.first().map(|r| r.finished_at.to_rfc3339());

        Ok(ApplicationListPage {
            items,
            total: total.0 as u64,
            excluded_ambient_count: excluded_ambient.0 as u64,
            page,
            page_size,
            cluster_ref: query.cluster_ref,
            run_id,
            last_assessed_at,
            cluster_summary: Default::default(),
            cluster_findings: Vec::new(),
        })
    }

    pub async fn get_application(
        &self,
        cluster_ref: &str,
        namespace: &str,
    ) -> Result<Option<ApplicationDetail>, DbError> {
        let row = sqlx::query_as::<_, AppDetailRow>(
            r#"
            SELECT
                aa.namespace,
                aa.application_name,
                aa.workload_components,
                aa.migration_candidate,
                c.cluster_ref,
                aa.mesh_revision,
                aa.discovery_label,
                aa.control_plane_namespace,
                aa.hostnames,
                aa.namespace_labels,
                aa.dataplane_mode,
                aa.ingress_gateway_namespace,
                aa.ingress_same_namespace,
                aa.workload_count,
                aa.readiness_pct,
                aa.risk_level,
                aa.blocker_count,
                aa.warning_count,
                aa.scores,
                aa.summary,
                aa.findings,
                aa.suggestions
            FROM application_assessments aa
            INNER JOIN clusters c ON c.id = aa.cluster_id
            INNER JOIN assessment_runs ar ON ar.id = aa.run_id
            WHERE c.cluster_ref = $1
              AND aa.namespace = $2
              AND ar.id = (
                SELECT id FROM assessment_runs
                WHERE cluster_id = c.id
                ORDER BY finished_at DESC
                LIMIT 1
              )
            "#,
        )
        .bind(cluster_ref)
        .bind(namespace)
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| r.into_detail()).transpose()
    }
}

async fn ensure_cluster_for_assessment(
    tx: &mut Transaction<'_, Postgres>,
    cluster_ref: &str,
) -> Result<Uuid, DbError> {
    let response = ambientor_dashboard::DashboardResponse {
        cluster_ref: cluster_ref.to_string(),
        cluster: ambientor_dashboard::ClusterDashboard {
            name: cluster_ref.to_string(),
            platform: "Kubernetes".into(),
            mesh_flavor: String::new(),
            istio_version: None,
            mesh_instance_count: 0,
            ambient_mesh_count: 0,
        },
        summary: Default::default(),
        mesh_instances: vec![],
        migration_savings: None,
        last_updated: Utc::now().to_rfc3339(),
        connection_namespace: None,
        connection_name: None,
        reachable: None,
        is_hub: None,
    };
    upsert_cluster(tx, &response).await
}

async fn insert_application(
    tx: &mut Transaction<'_, Postgres>,
    run_id: Uuid,
    cluster_id: Uuid,
    app: &ApplicationAssessmentRecord,
) -> Result<(), DbError> {
    sqlx::query(
        r#"
        INSERT INTO application_assessments (
            id, run_id, cluster_id, namespace, application_name, workload_components,
            migration_candidate, mesh_revision, discovery_label,
            control_plane_namespace, hostnames, namespace_labels, dataplane_mode,
            ingress_gateway_namespace, ingress_same_namespace, workload_count,
            readiness_pct, risk_level, blocker_count, warning_count,
            scores, summary, findings, suggestions, assessed_at
        )
        VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
            $18, $19, $20, $21, $22, $23, $24, NOW()
        )
        "#,
    )
    .bind(Uuid::new_v4())
    .bind(run_id)
    .bind(cluster_id)
    .bind(&app.namespace)
    .bind(&app.application_name)
    .bind(
        serde_json::to_value(&app.workload_components)
            .map_err(|e| DbError::Serialize(e.to_string()))?,
    )
    .bind(app.migration_candidate)
    .bind(&app.mesh_revision)
    .bind(&app.discovery_label)
    .bind(&app.control_plane_namespace)
    .bind(serde_json::to_value(&app.hostnames).map_err(|e| DbError::Serialize(e.to_string()))?)
    .bind(
        serde_json::to_value(&app.namespace_labels)
            .map_err(|e| DbError::Serialize(e.to_string()))?,
    )
    .bind(&app.dataplane_mode)
    .bind(&app.ingress_gateway_namespace)
    .bind(app.ingress_same_namespace)
    .bind(app.workload_count as i32)
    .bind(app.readiness_pct as i16)
    .bind(app.risk_level.as_db_str())
    .bind(app.blocker_count as i32)
    .bind(app.warning_count as i32)
    .bind(serde_json::to_value(&app.scores).map_err(|e| DbError::Serialize(e.to_string()))?)
    .bind(serde_json::to_value(&app.summary).map_err(|e| DbError::Serialize(e.to_string()))?)
    .bind(serde_json::to_value(&app.findings).map_err(|e| DbError::Serialize(e.to_string()))?)
    .bind(serde_json::to_value(&app.suggestions).map_err(|e| DbError::Serialize(e.to_string()))?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct AppRow {
    namespace: String,
    application_name: String,
    workload_components: serde_json::Value,
    migration_candidate: bool,
    cluster_ref: String,
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
    run_id: Uuid,
    finished_at: chrono::DateTime<Utc>,
}

impl AppRow {
    fn to_list_item(&self) -> ApplicationListItem {
        let namespace_labels: std::collections::BTreeMap<String, String> =
            serde_json::from_value(self.namespace_labels.clone()).unwrap_or_default();
        let dataplane_mode = effective_dataplane_mode(
            &self.dataplane_mode,
            &namespace_labels,
            self.mesh_revision.as_deref(),
            self.discovery_label.as_deref(),
        );
        ApplicationListItem {
            namespace: self.namespace.clone(),
            application_name: if self.application_name.is_empty() {
                self.namespace.clone()
            } else {
                self.application_name.clone()
            },
            workload_components: serde_json::from_value(self.workload_components.clone())
                .unwrap_or_default(),
            migration_candidate: self.migration_candidate,
            cluster_ref: self.cluster_ref.clone(),
            mesh_revision: self.mesh_revision.clone(),
            discovery_label: self.discovery_label.clone(),
            control_plane_namespace: self.control_plane_namespace.clone(),
            hostnames: serde_json::from_value(self.hostnames.clone()).unwrap_or_default(),
            namespace_labels,
            dataplane_mode,
            ingress_gateway_namespace: self.ingress_gateway_namespace.clone(),
            ingress_same_namespace: self.ingress_same_namespace,
            workload_count: self.workload_count as u32,
            readiness_pct: self.readiness_pct as u8,
            risk_level: RiskLevel::from_db_str(&self.risk_level),
            blocker_count: self.blocker_count as u32,
            warning_count: self.warning_count as u32,
        }
    }
}

#[derive(sqlx::FromRow)]
struct AppDetailRow {
    namespace: String,
    application_name: String,
    workload_components: serde_json::Value,
    migration_candidate: bool,
    cluster_ref: String,
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

impl AppDetailRow {
    fn into_detail(self) -> Result<ApplicationDetail, DbError> {
        let namespace_labels: std::collections::BTreeMap<String, String> =
            serde_json::from_value(self.namespace_labels)
                .map_err(|e| DbError::Serialize(e.to_string()))?;
        let dataplane_mode = effective_dataplane_mode(
            &self.dataplane_mode,
            &namespace_labels,
            self.mesh_revision.as_deref(),
            self.discovery_label.as_deref(),
        );
        let list = ApplicationListItem {
            namespace: self.namespace.clone(),
            application_name: if self.application_name.is_empty() {
                self.namespace.clone()
            } else {
                self.application_name
            },
            workload_components: serde_json::from_value(self.workload_components)
                .map_err(|e| DbError::Serialize(e.to_string()))?,
            migration_candidate: self.migration_candidate,
            cluster_ref: self.cluster_ref,
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
        };
        Ok(ApplicationDetail {
            list,
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

fn effective_dataplane_mode(
    stored: &str,
    labels: &std::collections::BTreeMap<String, String>,
    mesh_revision: Option<&str>,
    discovery_label: Option<&str>,
) -> String {
    if stored == "ambient" || stored == "sidecar" {
        return stored.to_string();
    }
    let derived = derive_dataplane_mode_from_stored(labels, mesh_revision, discovery_label);
    if derived != ambientor_dashboard::DataplaneMode::NotEnrolled {
        return derived.as_str().to_string();
    }
    if stored.is_empty() {
        derived.as_str().to_string()
    } else {
        stored.to_string()
    }
}

#[async_trait]
impl ApplicationAssessmentStore for ApplicationAssessmentRepository {
    async fn replace_run(&self, run: &ClusterAssessmentRun) -> Result<Uuid, DbError> {
        ApplicationAssessmentRepository::replace_run(self, run).await
    }

    async fn list_applications(
        &self,
        query: ApplicationListQuery,
    ) -> Result<ApplicationListPage, DbError> {
        ApplicationAssessmentRepository::list_applications(self, query).await
    }

    async fn get_application(
        &self,
        cluster_ref: &str,
        namespace: &str,
    ) -> Result<Option<ApplicationDetail>, DbError> {
        ApplicationAssessmentRepository::get_application(self, cluster_ref, namespace).await
    }
}
