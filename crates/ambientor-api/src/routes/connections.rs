use std::sync::Arc;

use ambientor_core::scoring::compute_scores;
use ambientor_db::StoredAssessment;
use ambientor_k8s::{
    K8sClient, client_for_connection, connection_cluster_ref, verify_connectivity,
};
use ambientor_mesh::backend::backend_for_flavor;
use ambientor_scan::default_registry;
use ambientor_types::{ClusterConnection, FindingSummary};
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use kube::Api;
use serde::Serialize;

use crate::routes::applications::persist_assessment_from_findings;
use crate::routes::assess::{AssessRequest, AssessResponse};
use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionListItem {
    pub name: String,
    pub namespace: String,
    pub display_name: String,
    pub phase: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_sync_time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready_message: Option<String>,
    pub hub: bool,
}

pub async fn list_connections(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<Vec<ConnectionListItem>>, (StatusCode, String)> {
    let hub = hub_client().await?;
    let api = Api::<ClusterConnection>::all(hub.client.clone());
    let list = api.list(&Default::default()).await.map_err(internal)?;
    let items = list
        .items
        .into_iter()
        .filter_map(connection_to_item)
        .collect();
    Ok(Json(items))
}

pub async fn assess_connection(
    State(state): State<Arc<AppState>>,
    Path((namespace, name)): Path<(String, String)>,
    Json(body): Json<AssessRequest>,
) -> Result<Json<AssessResponse>, (StatusCode, String)> {
    let hub = hub_client().await?;
    let api = Api::<ClusterConnection>::namespaced(hub.client.clone(), &namespace);
    let conn = api.get(&name).await.map_err(map_kube_err)?;
    if conn.spec.hub {
        return Err((
            StatusCode::BAD_REQUEST,
            "cannot assess hub-local connection; use POST /api/v1/assess".into(),
        ));
    }

    let remote = client_for_connection(&hub.client, &conn)
        .await
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    verify_connectivity(&remote.client).await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("remote cluster unreachable: {e}"),
        )
    })?;

    let namespace_filter = body.namespace.as_deref();
    let platform = ambientor_k8s::detect_platform(&remote.client)
        .await
        .map_err(internal)?;
    let backend = backend_for_flavor(platform.mesh_flavor);
    let mut ctx = backend
        .build_rule_context(&remote.client)
        .await
        .map_err(internal)?;
    if let Ok(Some(ver)) = backend.detect_version(&remote.client).await {
        ctx.mesh_version = Some(ver);
    }

    let registry = default_registry();
    let findings = registry.evaluate_all(&ctx);
    let scores = compute_scores(&findings);
    let summary = FindingSummary::from_findings(&findings);
    let cluster_ref = connection_cluster_ref(&namespace, &name);

    state.sse.write().await.publish(
        "assessment",
        &serde_json::json!({
            "phase": "completed",
            "findingCount": findings.len(),
            "clusterRef": cluster_ref,
        }),
    );

    if let Some(repo) = state.scan_store() {
        let payload = StoredAssessment {
            findings: findings.clone(),
            scores: scores.clone(),
            summary: summary.clone(),
            source: Some(format!("connection:{namespace}/{name}")),
            assessment_name: None,
        };
        if let Err(e) = repo
            .record_completed(&cluster_ref, namespace_filter, &payload)
            .await
        {
            tracing::warn!(error = %e, cluster_ref = %cluster_ref, "failed to persist remote scan");
        }
    }

    if let Err(e) = persist_assessment_from_findings(
        state.as_ref(),
        &remote.client,
        &cluster_ref,
        &ctx,
        &findings,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            cluster_ref = %cluster_ref,
            "failed to persist remote application assessments"
        );
    }

    Ok(Json(AssessResponse {
        findings,
        scores,
        summary,
        application_count: 0,
    }))
}

fn connection_to_item(conn: ClusterConnection) -> Option<ConnectionListItem> {
    let name = conn.metadata.name?;
    let namespace = conn.metadata.namespace.unwrap_or_else(|| "default".into());
    let status = conn.status.unwrap_or_default();
    let ready_message = status
        .conditions
        .iter()
        .find(|c| c.r#type == "Ready")
        .and_then(|c| c.message.clone());
    Some(ConnectionListItem {
        name,
        namespace,
        display_name: conn.spec.display_name,
        phase: status.phase,
        last_sync_time: status.last_sync_time.map(|t| t.to_rfc3339()),
        ready_message,
        hub: conn.spec.hub,
    })
}

async fn hub_client() -> Result<K8sClient, (StatusCode, String)> {
    K8sClient::in_cluster()
        .await
        .or(K8sClient::from_kubeconfig(None).await)
        .map_err(internal)
}

fn internal(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

fn map_kube_err(e: kube::Error) -> (StatusCode, String) {
    match e {
        kube::Error::Api(err) if err.code == 404 => (StatusCode::NOT_FOUND, err.to_string()),
        other => internal(other),
    }
}
