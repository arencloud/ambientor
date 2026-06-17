use std::sync::Arc;

use ambientor_db::{
    ApplicationListQuery, assessment_sync::persist_full_assessment, cluster_ref_from_env,
};
use ambientor_mesh::inventory::CollectedInventory;
use ambientor_dashboard::ApplicationListPage;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct ApplicationsQuery {
    #[serde(rename = "clusterRef")]
    pub cluster_ref: Option<String>,
    pub q: Option<String>,
    #[serde(rename = "riskLevel")]
    pub risk_level: Option<String>,
    #[serde(rename = "meshRevision")]
    pub mesh_revision: Option<String>,
    /// When true (default), hide namespaces already on ambient dataplane.
    #[serde(rename = "migrationCandidatesOnly", default = "default_migration_candidates_only")]
    pub migration_candidates_only: bool,
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(rename = "pageSize", default = "default_page_size")]
    pub page_size: u32,
}

fn default_page() -> u32 {
    1
}

fn default_page_size() -> u32 {
    50
}

fn default_migration_candidates_only() -> bool {
    true
}

pub async fn list_applications(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ApplicationsQuery>,
) -> Result<Json<ApplicationListPage>, (StatusCode, String)> {
    let store = state.applications_store().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "DATABASE_URL not configured".into(),
    ))?;

    let cluster_ref = query
        .cluster_ref
        .filter(|s| !s.is_empty())
        .unwrap_or_else(cluster_ref_from_env);

    let page = store
        .list_applications(ApplicationListQuery {
            cluster_ref,
            search: query.q,
            risk_level: query.risk_level,
            mesh_revision: query.mesh_revision,
            migration_candidates_only: query.migration_candidates_only,
            page: query.page,
            page_size: query.page_size,
        })
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(page))
}

#[derive(Debug, Deserialize)]
pub struct ApplicationDetailQuery {
    #[serde(rename = "clusterRef")]
    pub cluster_ref: Option<String>,
}

pub async fn get_application(
    State(state): State<Arc<AppState>>,
    Path(namespace): Path<String>,
    Query(query): Query<ApplicationDetailQuery>,
) -> Result<Json<ambientor_dashboard::ApplicationDetail>, (StatusCode, String)> {
    let store = state.applications_store().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "DATABASE_URL not configured".into(),
    ))?;

    let cluster_ref = query
        .cluster_ref
        .filter(|s| !s.is_empty())
        .unwrap_or_else(cluster_ref_from_env);

    let detail = store
        .get_application(&cluster_ref, &namespace)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "application not found".into()))?;

    Ok(Json(detail))
}

pub async fn persist_assessment_from_inventory(
    state: &AppState,
    hub: Option<&kube::Client>,
    spoke: &kube::Client,
    cluster_ref: &str,
    inventory: &CollectedInventory,
    findings: &[ambientor_types::Finding],
) -> Result<usize, String> {
    let apps = state
        .applications_store()
        .ok_or_else(|| "DATABASE_URL not configured".to_string())?;
    let dash = state
        .dashboard_store()
        .ok_or_else(|| "DATABASE_URL not configured".to_string())?;

    persist_full_assessment(
        apps.as_ref(),
        dash.as_ref(),
        hub,
        spoke,
        cluster_ref,
        inventory,
        findings,
    )
    .await
    .map_err(|e| e.to_string())
}
