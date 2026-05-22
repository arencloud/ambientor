use std::sync::Arc;

use ambientor_core::scoring::compute_scores;
use ambientor_k8s::K8sClient;
use ambientor_mesh::backend::backend_for_flavor;
use ambientor_scan::default_registry;
use ambientor_types::FindingSummary;
use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Deserialize)]
pub struct AssessRequest {
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Serialize)]
pub struct AssessResponse {
    pub findings: Vec<ambientor_types::Finding>,
    pub scores: ambientor_types::AssessmentScores,
    pub summary: FindingSummary,
}

pub async fn assess(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AssessRequest>,
) -> Result<Json<AssessResponse>, (axum::http::StatusCode, String)> {
    let _namespace_filter = body.namespace;
    let k8s = K8sClient::in_cluster()
        .await
        .or(K8sClient::from_kubeconfig(None).await)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let platform = ambientor_k8s::detect_platform(&k8s.client)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let backend = backend_for_flavor(platform.mesh_flavor);
    let mut ctx = backend
        .build_rule_context(&k8s.client)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Ok(Some(ver)) = backend.detect_version(&k8s.client).await {
        ctx.mesh_version = Some(ver);
    }

    let registry = default_registry();
    let findings = registry.evaluate_all(&ctx);
    let scores = compute_scores(&findings);
    let summary = FindingSummary::from_findings(&findings);

    state.sse.write().await.publish(
        "assessment",
        &serde_json::json!({ "phase": "completed", "findingCount": findings.len() }),
    );

    Ok(Json(AssessResponse {
        findings,
        scores,
        summary,
    }))
}
