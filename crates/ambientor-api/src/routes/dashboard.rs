use std::sync::Arc;

use ambientor_dashboard::{AssessmentFindingsOverrides, DashboardResponse, build_dashboard};
use ambientor_db::{cluster_ref_from_env, load_assessment_findings_overrides};
use ambientor_k8s::{
    K8sClient, client_for_connection, parse_connection_cluster_ref, verify_connectivity,
};
use ambientor_types::{AmbientAssessment, ClusterConnection};
use axum::{
    Json,
    extract::{Query, State},
};
use kube::{Api, api::ListParams};
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
) -> Result<Json<ambientor_dashboard::FleetDashboardResponse>, (axum::http::StatusCode, String)> {
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
    let fleet = ambientor_dashboard::FleetDashboardResponse {
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

/// Recompute dashboard from the cluster, persist, and notify SSE subscribers.
pub async fn refresh_and_notify(state: &AppState, cluster_ref: &str) {
    match compute_and_persist_live(state, cluster_ref).await {
        Ok(_) => {
            state.sse.write().await.publish(
                "dashboard",
                &serde_json::json!({ "clusterRef": cluster_ref }),
            );
        }
        Err((status, msg)) => {
            tracing::warn!(
                status = ?status,
                error = %msg,
                cluster_ref = %cluster_ref,
                "dashboard refresh failed"
            );
        }
    }
}

async fn compute_and_persist_live(
    state: &AppState,
    cluster_ref: &str,
) -> Result<DashboardResponse, (axum::http::StatusCode, String)> {
    let hub = k8s_client().await?;
    let (client, spoke_meta) = resolve_dashboard_client(&hub, cluster_ref)
        .await
        .map_err(internal)?;

    let overrides = load_findings_overrides(state, &client, cluster_ref).await;
    let mut response = build_dashboard(&client, cluster_ref, overrides.as_ref())
        .await
        .map_err(internal)?;
    if let Some(meta) = spoke_meta {
        response.cluster.name = meta.display_name;
        response.connection_namespace = Some(meta.connection_namespace);
        response.connection_name = Some(meta.connection_name);
        response.is_hub = Some(false);
        response.reachable = Some(true);
    } else {
        response.is_hub = Some(true);
        response.reachable = Some(true);
    }

    if let Some(store) = state.dashboard_store() {
        if let Err(e) = store.sync_snapshot(&response).await {
            tracing::warn!(error = %e, cluster_ref = %cluster_ref, "dashboard sync to database failed");
        }
    }

    Ok(response)
}

struct SpokeDashboardMeta {
    display_name: String,
    connection_namespace: String,
    connection_name: String,
}

async fn resolve_dashboard_client(
    hub: &K8sClient,
    cluster_ref: &str,
) -> Result<(kube::Client, Option<SpokeDashboardMeta>), String> {
    let Some((ns, name)) = parse_connection_cluster_ref(cluster_ref) else {
        return Ok((hub.client.clone(), None));
    };

    let api: Api<ClusterConnection> = Api::namespaced(hub.client.clone(), ns);
    let conn = api
        .get(name)
        .await
        .map_err(|e| format!("ClusterConnection {ns}/{name}: {e}"))?;
    if conn.spec.hub {
        return Err(format!(
            "cluster_ref {cluster_ref} is hub-local; omit clusterRef for hub dashboard"
        ));
    }

    let display_name = conn.spec.display_name.clone();
    let remote = client_for_connection(&hub.client, &conn)
        .await
        .map_err(|e| format!("remote client for {cluster_ref}: {e}"))?;
    verify_connectivity(&remote.client)
        .await
        .map_err(|e| format!("remote cluster {cluster_ref} unreachable: {e}"))?;

    Ok((
        remote.client,
        Some(SpokeDashboardMeta {
            display_name,
            connection_namespace: ns.to_string(),
            connection_name: name.to_string(),
        }),
    ))
}

async fn load_findings_overrides(
    state: &AppState,
    client: &kube::Client,
    cluster_ref: &str,
) -> Option<AssessmentFindingsOverrides> {
    let scan_repo = state.scan_store()?;
    let names = empty_findings_assessment_names(client).await.ok()?;
    if names.is_empty() {
        return None;
    }
    load_assessment_findings_overrides(scan_repo.as_ref(), cluster_ref, &names)
        .await
        .ok()
        .filter(|m| !m.is_empty())
}

async fn empty_findings_assessment_names(
    client: &kube::Client,
) -> Result<Vec<String>, kube::Error> {
    let api: Api<AmbientAssessment> = Api::all(client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list
        .items
        .into_iter()
        .filter_map(|a| {
            let name = a.metadata.name?;
            let empty = a
                .status
                .as_ref()
                .is_some_and(|s| s.findings.is_empty());
            empty.then_some(name)
        })
        .collect())
}
