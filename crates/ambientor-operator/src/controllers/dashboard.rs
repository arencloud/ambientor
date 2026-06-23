use std::sync::Arc;
use std::time::Duration;

use ambientor_dashboard::{AssessmentFindingsOverrides, DashboardResponse, build_dashboard};
use ambientor_db::{
    DashboardStore, ScanStore, cluster_ref_from_env, load_assessment_findings_overrides,
};
use ambientor_k8s::{
    client_for_connection, connection_cluster_ref, is_remote_cluster_ref,
    parse_connection_cluster_ref, verify_connectivity,
};
use ambientor_types::{AmbientAssessment, ClusterConnection};
use kube::{Api, Client, api::ListParams};
use tracing::info;

pub async fn run(
    client: Client,
    store: Option<Arc<dyn DashboardStore>>,
    scan_repo: Option<Arc<dyn ScanStore>>,
) {
    let Some(store) = store else {
        tracing::debug!("DATABASE_URL not set; dashboard sync disabled");
        return;
    };

    let interval_secs = std::env::var("AMBIENTOR_DASHBOARD_SYNC_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(60);
    info!(interval_secs, "dashboard sync loop started");
    let store = store.clone();
    loop {
        sync_hub_dashboard(&client, store.as_ref(), scan_repo.as_deref()).await;
        sync_spoke_dashboards(&client, store.as_ref(), scan_repo.as_deref()).await;
        tokio::time::sleep(Duration::from_secs(interval_secs)).await;
    }
}

/// Recompute hub dashboard and persist to Postgres.
#[allow(dead_code)]
pub async fn sync_hub_now(
    client: &Client,
    store: &dyn DashboardStore,
    scan_repo: Option<&dyn ScanStore>,
) {
    sync_hub_dashboard(client, store, scan_repo).await;
}

/// Refresh dashboard snapshot for hub or a single spoke `cluster_ref` after rollout.
pub async fn sync_cluster_ref_now(
    hub: &Client,
    store: &dyn DashboardStore,
    scan_repo: Option<&dyn ScanStore>,
    cluster_ref: Option<&str>,
) {
    let Some(cluster_ref) = cluster_ref.map(str::trim).filter(|s| !s.is_empty()) else {
        sync_hub_dashboard(hub, store, scan_repo).await;
        return;
    };
    if is_remote_cluster_ref(cluster_ref) {
        sync_spoke_dashboard(hub, store, scan_repo, cluster_ref).await;
    } else {
        sync_hub_dashboard(hub, store, scan_repo).await;
    }
}

async fn sync_hub_dashboard(
    client: &Client,
    store: &dyn DashboardStore,
    scan_repo: Option<&dyn ScanStore>,
) {
    let cluster_ref = cluster_ref_from_env();
    match build_dashboard_response(client, &cluster_ref, scan_repo, client).await {
        Ok(mut response) => {
            response.is_hub = Some(true);
            response.reachable = Some(true);
            if let Err(e) = store.sync_snapshot(&response).await {
                tracing::warn!(
                    error = %e,
                    cluster_ref = %cluster_ref,
                    "hub dashboard sync to database failed"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                cluster_ref = %cluster_ref,
                "hub dashboard compute failed"
            );
        }
    }
}

async fn sync_spoke_dashboards(
    hub: &Client,
    store: &dyn DashboardStore,
    scan_repo: Option<&dyn ScanStore>,
) {
    let api: Api<ClusterConnection> = Api::all(hub.clone());
    let list = match api.list(&ListParams::default()).await {
        Ok(list) => list,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list ClusterConnection for dashboard sync");
            return;
        }
    };

    for conn in list.items {
        if conn.spec.hub {
            continue;
        }
        let Some(name) = conn.metadata.name.clone() else {
            continue;
        };
        let ns = conn
            .metadata
            .namespace
            .clone()
            .unwrap_or_else(|| "default".into());
        let cluster_ref = connection_cluster_ref(&ns, &name);
        sync_spoke_dashboard(hub, store, scan_repo, &cluster_ref).await;
    }
}

async fn sync_spoke_dashboard(
    hub: &Client,
    store: &dyn DashboardStore,
    scan_repo: Option<&dyn ScanStore>,
    cluster_ref: &str,
) {
    let Some((ns, name)) = parse_connection_cluster_ref(cluster_ref) else {
        return;
    };
    let api: Api<ClusterConnection> = Api::namespaced(hub.clone(), ns);
    let conn = match api.get(name).await {
        Ok(conn) => conn,
        Err(e) => {
            tracing::warn!(
                error = %e,
                cluster_ref = %cluster_ref,
                "spoke dashboard sync skipped: connection not found"
            );
            return;
        }
    };
    let display_name = conn.spec.display_name.clone();

    let remote = match client_for_connection(hub, &conn).await {
        Ok(remote) => remote,
        Err(e) => {
            tracing::warn!(
                error = %e,
                cluster_ref = %cluster_ref,
                "spoke dashboard sync skipped: invalid connection"
            );
            let response = unreachable_spoke_response(cluster_ref, &display_name, ns, name);
            if let Err(e) = store.sync_snapshot(&response).await {
                tracing::warn!(error = %e, cluster_ref = %cluster_ref, "spoke unreachable registry update failed");
            }
            return;
        }
    };

    if let Err(e) = verify_connectivity(&remote.client).await {
        tracing::warn!(
            error = %e,
            cluster_ref = %cluster_ref,
            "spoke dashboard sync skipped: unreachable"
        );
        let response = unreachable_spoke_response(cluster_ref, &display_name, ns, name);
        if let Err(e) = store.sync_snapshot(&response).await {
            tracing::warn!(error = %e, cluster_ref = %cluster_ref, "spoke unreachable registry update failed");
        }
        return;
    }

    match build_dashboard_response(&remote.client, cluster_ref, scan_repo, hub).await {
        Ok(mut response) => {
            response.cluster.name = display_name;
            response.connection_namespace = Some(ns.to_string());
            response.connection_name = Some(name.to_string());
            response.is_hub = Some(false);
            response.reachable = Some(true);
            if let Err(e) = store.sync_snapshot(&response).await {
                tracing::warn!(
                    error = %e,
                    cluster_ref = %cluster_ref,
                    "spoke dashboard sync to database failed"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                cluster_ref = %cluster_ref,
                "spoke dashboard compute failed"
            );
        }
    }
}

async fn build_dashboard_response(
    client: &Client,
    cluster_ref: &str,
    scan_repo: Option<&dyn ScanStore>,
    rollout_client: &Client,
) -> anyhow::Result<DashboardResponse> {
    let overrides = load_findings_overrides(client, scan_repo, cluster_ref).await;
    let mut response = build_dashboard(
        client,
        cluster_ref,
        overrides.as_ref(),
        Some(rollout_client),
    )
    .await?;
    if response.connection_namespace.is_none()
        && let Some((ns, name)) = parse_connection_cluster_ref(cluster_ref)
    {
        response.connection_namespace = Some(ns.into());
        response.connection_name = Some(name.into());
    }
    Ok(response)
}

async fn load_findings_overrides(
    client: &Client,
    scan_repo: Option<&dyn ScanStore>,
    cluster_ref: &str,
) -> Option<AssessmentFindingsOverrides> {
    let scan_repo = scan_repo?;
    let names = empty_findings_assessment_names(client).await.ok()?;
    if names.is_empty() {
        return None;
    }
    load_assessment_findings_overrides(scan_repo, cluster_ref, &names)
        .await
        .ok()
        .filter(|m| !m.is_empty())
}

async fn empty_findings_assessment_names(client: &Client) -> anyhow::Result<Vec<String>> {
    let api: Api<AmbientAssessment> = Api::all(client.clone());
    let list = api.list(&ListParams::default()).await?;
    Ok(list
        .items
        .into_iter()
        .filter_map(|a| {
            let name = a.metadata.name?;
            let empty = a.status.as_ref().is_some_and(|s| s.findings.is_empty());
            empty.then_some(name)
        })
        .collect())
}

fn unreachable_spoke_response(
    cluster_ref: &str,
    display_name: &str,
    connection_namespace: &str,
    connection_name: &str,
) -> DashboardResponse {
    DashboardResponse {
        cluster_ref: cluster_ref.to_string(),
        cluster: ambientor_dashboard::ClusterDashboard {
            name: display_name.to_string(),
            platform: "Kubernetes".into(),
            mesh_flavor: String::new(),
            istio_version: None,
            mesh_instance_count: 0,
            ambient_mesh_count: 0,
        },
        summary: Default::default(),
        mesh_instances: vec![],
        migration_savings: None,
        last_updated: chrono::Utc::now().to_rfc3339(),
        connection_namespace: Some(connection_namespace.to_string()),
        connection_name: Some(connection_name.to_string()),
        reachable: Some(false),
        is_hub: Some(false),
    }
}
