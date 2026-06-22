use std::sync::Arc;

use ambientor_dashboard::{
    apply_cluster_ref_metadata, AssessmentFindingsOverrides, ClusterDashboard, DashboardResponse,
    FleetClusterDashboard, FleetDashboardResponse, StatusCounts, build_dashboard,
    list_rollout_ns_status, overlay_fleet_rollout_status, overlay_rollout_status,
};
use ambientor_db::{cluster_ref_from_env, load_assessment_findings_overrides};
use ambientor_k8s::{
    K8sClient, client_for_connection, connection_cluster_ref, connection_display_names,
    parse_connection_cluster_ref, resolve_cluster_display_name, verify_connectivity,
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
    /// When true, return only persisted DB snapshots (no live cluster recompute).
    #[serde(default, rename = "dbOnly")]
    pub db_only: bool,
    /// When true with `dbOnly`, rebuild from latest assessment rows then return snapshot.
    #[serde(default, rename = "rebuildAssess")]
    pub rebuild_assess: bool,
}

pub async fn get_dashboard(
    State(state): State<Arc<AppState>>,
    Query(query): Query<DashboardQuery>,
) -> Result<Json<DashboardResponse>, (axum::http::StatusCode, String)> {
    let cluster_ref = query
        .cluster_ref
        .filter(|s| !s.is_empty())
        .unwrap_or_else(cluster_ref_from_env);

    if query.db_only {
        let store = state.dashboard_store().ok_or((
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "DATABASE_URL not configured".into(),
        ))?;
        if query.rebuild_assess {
            if let Ok(Some(rebuilt)) = store.rebuild_from_latest_assessment(&cluster_ref).await {
                if let Err(e) = store.sync_snapshot(&rebuilt).await {
                    tracing::warn!(error = %e, "failed to sync rebuilt dashboard snapshot");
                } else {
                    return Ok(Json(rebuilt));
                }
            }
        }
        if let Ok(Some(cached)) = store.load_by_cluster_ref(&cluster_ref).await {
            return Ok(Json(cached));
        }
        return Err((
            axum::http::StatusCode::NOT_FOUND,
            "no dashboard snapshot in database for this cluster; run assessment".into(),
        ));
    }

    if let Some(store) = state.dashboard_store() {
        if let Ok(Some(mut cached)) = store.load_by_cluster_ref(&cluster_ref).await {
            if let Ok(hub) = k8s_client().await {
                if let Ok(rollouts) =
                    list_rollout_ns_status(&hub.client, &cluster_ref).await
                {
                    overlay_rollout_status(&mut cached, &rollouts);
                }
            }
            if query.fresh {
                spawn_background_cluster_refresh(state.clone(), cluster_ref.clone());
                return Ok(Json(cached));
            }
            if !store
                .is_snapshot_stale(&cluster_ref)
                .await
                .unwrap_or(true)
            {
                return Ok(Json(cached));
            }
        }

        if !query.fresh {
            if let Ok(Some(mut rebuilt)) = store.rebuild_from_latest_assessment(&cluster_ref).await
            {
                if let Ok(hub) = k8s_client().await {
                    if let Ok(rollouts) =
                        list_rollout_ns_status(&hub.client, &cluster_ref).await
                    {
                        overlay_rollout_status(&mut rebuilt, &rollouts);
                    }
                }
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

#[derive(Debug, Deserialize)]
pub struct FleetDashboardQuery {
    #[serde(default)]
    pub fresh: bool,
    #[serde(default, rename = "dbOnly")]
    pub db_only: bool,
    #[serde(default, rename = "rebuildAssess")]
    pub rebuild_assess: bool,
}

pub async fn get_fleet_dashboard(
    State(state): State<Arc<AppState>>,
    Query(query): Query<FleetDashboardQuery>,
) -> Result<Json<ambientor_dashboard::FleetDashboardResponse>, (axum::http::StatusCode, String)> {
    let store = state.dashboard_store().ok_or((
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "DATABASE_URL not configured".into(),
    ))?;

    if query.db_only {
        if query.rebuild_assess {
            store
                .rebuild_all_from_latest_assessments()
                .await
                .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        }
        if let Some(fleet) = store
            .load_fleet()
            .await
            .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        {
            return Ok(Json(fleet));
        }
        return Err((
            axum::http::StatusCode::NOT_FOUND,
            "no fleet dashboard snapshots in database; run assessment on clusters".into(),
        ));
    }

    let hub = k8s_client().await?;
    let mut fleet = if let Some(fleet) = store
        .load_fleet()
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    {
        fleet
    } else {
        let cluster_ref = cluster_ref_from_env();
        let response = compute_and_persist_live(&state, &cluster_ref).await?;
        let summary = response.summary.clone();
        FleetDashboardResponse {
            summary: summary.clone(),
            clusters: vec![FleetClusterDashboard {
                cluster_ref: response.cluster_ref,
                cluster: response.cluster,
                summary,
                mesh_instances: response.mesh_instances,
                last_updated: response.last_updated.clone(),
            }],
            last_updated: response.last_updated,
        }
    };

    let mut rollouts_by_cluster = std::collections::HashMap::new();
    for cluster in &fleet.clusters {
        if let Ok(rollouts) = list_rollout_ns_status(&hub.client, &cluster.cluster_ref).await {
            if !rollouts.is_empty() {
                rollouts_by_cluster.insert(cluster.cluster_ref.clone(), rollouts);
            }
        }
    }
    overlay_fleet_rollout_status(&mut fleet, &rollouts_by_cluster);

    fleet = merge_fleet_with_connections(fleet, &hub.client).await;
    enrich_fleet_display_names(&mut fleet, &hub.client).await;

    if query.fresh {
        spawn_background_fleet_refresh(state);
    }

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

async fn refresh_fleet_live(state: &AppState) -> Result<(), String> {
    let hub = k8s_client().await.map_err(|(_, msg)| msg)?;
    let hub_ref = cluster_ref_from_env();
    compute_and_persist_live(state, &hub_ref)
        .await
        .map_err(|(_, msg)| msg)?;
    let api: Api<ClusterConnection> = Api::all(hub.client.clone());
    if let Ok(list) = api.list(&ListParams::default()).await {
        for conn in list.items {
            if conn.spec.hub {
                continue;
            }
            let Some(name) = conn.metadata.name else {
                continue;
            };
            let ns = conn
                .metadata
                .namespace
                .unwrap_or_else(|| "default".into());
            let cluster_ref = connection_cluster_ref(&ns, &name);
            if let Err((_, msg)) = compute_and_persist_live(state, &cluster_ref).await {
                tracing::warn!(cluster_ref = %cluster_ref, error = %msg, "spoke dashboard refresh failed");
            }
        }
    }
    Ok(())
}

fn spawn_background_fleet_refresh(state: Arc<AppState>) {
    tokio::spawn(async move {
        match refresh_fleet_live(&state).await {
            Ok(()) => {
                state.sse.write().await.publish(
                    "dashboard",
                    &serde_json::json!({ "scope": "fleet" }),
                );
            }
            Err(e) => {
                tracing::warn!(error = %e, "background fleet dashboard refresh failed");
            }
        }
    });
}

fn spawn_background_cluster_refresh(state: Arc<AppState>, cluster_ref: String) {
    tokio::spawn(async move {
        refresh_and_notify(&state, &cluster_ref).await;
    });
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
    let mut response = build_dashboard(
        &client,
        cluster_ref,
        overrides.as_ref(),
        Some(&hub.client),
    )
    .await
    .map_err(internal)?;
    response.cluster.name =
        resolve_cluster_display_name(Some(&hub.client), cluster_ref, &response.cluster.name).await;
    apply_cluster_ref_metadata(cluster_ref, &mut response);
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
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(kube::Error::Api(e)) if e.code == 404 => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
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

async fn merge_fleet_with_connections(
    mut fleet: FleetDashboardResponse,
    hub: &kube::Client,
) -> FleetDashboardResponse {
    let api: Api<ClusterConnection> = Api::all(hub.clone());
    let Ok(list) = api.list(&ListParams::default()).await else {
        return fleet;
    };
    let hub_ref = cluster_ref_from_env();
    let mut seen: std::collections::HashSet<String> = fleet
        .clusters
        .iter()
        .map(|c| c.cluster_ref.clone())
        .collect();

    for conn in list.items {
        let Some(name) = conn.metadata.name else {
            continue;
        };
        let ns = conn
            .metadata
            .namespace
            .unwrap_or_else(|| "default".into());
        let cluster_ref = if conn.spec.hub {
            hub_ref.clone()
        } else {
            connection_cluster_ref(&ns, &name)
        };
        if !seen.insert(cluster_ref.clone()) {
            continue;
        }
        fleet.clusters.push(FleetClusterDashboard {
            cluster_ref,
            cluster: ClusterDashboard {
                name: conn.spec.display_name.clone(),
                platform: String::new(),
                mesh_flavor: String::new(),
                istio_version: None,
                mesh_instance_count: 0,
                ambient_mesh_count: 0,
            },
            summary: StatusCounts::default(),
            mesh_instances: Vec::new(),
            last_updated: fleet.last_updated.clone(),
        });
    }
    fleet
}

async fn enrich_fleet_display_names(
    fleet: &mut FleetDashboardResponse,
    hub: &kube::Client,
) {
    let hub_ref = cluster_ref_from_env();
    let Ok(names) = connection_display_names(hub, &hub_ref).await else {
        return;
    };
    for entry in &mut fleet.clusters {
        if let Some(display) = names.get(&entry.cluster_ref) {
            entry.cluster.name = display.clone();
        }
    }
}
