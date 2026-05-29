use std::sync::Arc;

use ambientor_dashboard::{DashboardResponse, FleetDashboardResponse, build_dashboard};
use ambientor_db::cluster_ref_from_env;
use axum::{
    Json,
    extract::{Query, State},
};
use serde::Deserialize;

use crate::state::AppState;

use super::plans::{internal, k8s_client};

#[derive(Debug, Deserialize)]
pub struct DashboardQuery {
    #[serde(rename = "clusterRef")]
    pub cluster_ref: Option<String>,
    /// When true, rebuild dashboard from latest assessment in DB (or live cluster).
    #[serde(default)]
    pub fresh: bool,
}

pub async fn get_dashboard(
    State(state): State<Arc<AppState>>,
    Query(query): Query<DashboardQuery>,
) -> Result<Json<DashboardResponse>, (axum::http::StatusCode, String)> {
    let cluster_ref = query
        .cluster_ref
        .filter(|s| !s.is_empty())
        .unwrap_or_else(cluster_ref_from_env);

    if let Some(store) = state.dashboard_store() {
        let stale = query.fresh
            || store
                .is_snapshot_stale(&cluster_ref)
                .await
                .unwrap_or(true);

        if !stale {
            if let Ok(Some(cached)) = store.load_by_cluster_ref(&cluster_ref).await {
                return Ok(Json(cached));
            }
        }

        if stale {
            if let Ok(Some(rebuilt)) = store.rebuild_from_latest_assessment(&cluster_ref).await {
                if let Err(e) = store.sync_snapshot(&rebuilt).await {
                    tracing::warn!(error = %e, "failed to refresh dashboard snapshot from assessment");
                } else {
                    return Ok(Json(rebuilt));
                }
            }
        }
    }

    let response = compute_and_persist_live(&state, &cluster_ref).await?;
    Ok(Json(response))
}

pub async fn get_fleet_dashboard(
    State(state): State<Arc<AppState>>,
) -> Result<Json<FleetDashboardResponse>, (axum::http::StatusCode, String)> {
    let store = state.dashboard_store().ok_or((
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "DATABASE_URL not configured".into(),
    ))?;

    if let Some(fleet) = store
        .load_fleet()
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    {
        return Ok(Json(fleet));
    }

    let cluster_ref = cluster_ref_from_env();
    let response = compute_and_persist_live(&state, &cluster_ref).await?;
    let summary = response.summary.clone();
    let fleet = FleetDashboardResponse {
        summary: summary.clone(),
        clusters: vec![ambientor_dashboard::FleetClusterDashboard {
            cluster_ref: response.cluster_ref,
            cluster: response.cluster,
            summary,
            mesh_instances: response.mesh_instances,
            last_updated: response.last_updated.clone(),
        }],
        last_updated: response.last_updated,
    };
    Ok(Json(fleet))
}

async fn compute_and_persist_live(
    state: &AppState,
    cluster_ref: &str,
) -> Result<DashboardResponse, (axum::http::StatusCode, String)> {
    let k8s = k8s_client().await?;
    let response = build_dashboard(&k8s.client, cluster_ref)
        .await
        .map_err(internal)?;

    if let Some(store) = state.dashboard_store() {
        if let Err(e) = store.sync_snapshot(&response).await {
            tracing::warn!(error = %e, cluster_ref = %cluster_ref, "dashboard sync to database failed");
        }
    }

    Ok(response)
}