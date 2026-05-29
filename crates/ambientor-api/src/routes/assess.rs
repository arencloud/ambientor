use std::sync::Arc;

use ambientor_core::scoring::compute_scores;
use ambientor_db::{StoredAssessment, cluster_ref_from_env};
use ambientor_k8s::K8sClient;
use ambientor_mesh::backend::backend_for_flavor;
use ambientor_scan::default_registry;
use ambientor_types::FindingSummary;
use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::routes::applications::persist_assessment_from_findings;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct AssessRequest {
    #[serde(default)]
    pub namespace: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessResponse {
    pub findings: Vec<ambientor_types::Finding>,
    pub scores: ambientor_types::AssessmentScores,
    pub summary: FindingSummary,
    pub application_count: usize,
}

pub async fn assess(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AssessRequest>,
) -> Result<Json<AssessResponse>, (axum::http::StatusCode, String)> {
    let namespace_filter = body.namespace.as_deref();
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
    let cluster_ref = cluster_ref_from_env();

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
            source: Some("api".into()),
            assessment_name: None,
        };
        if let Err(e) = repo
            .record_completed(&cluster_ref, namespace_filter, &payload)
            .await
        {
            tracing::warn!(error = %e, "failed to persist scan run");
        }
    }

    let application_count = persist_assessment_from_findings(
        state.as_ref(),
        &k8s.client,
        &cluster_ref,
        &ctx,
        &findings,
    )
    .await
    .unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to persist application assessments");
        0
    });

    Ok(Json(AssessResponse {
        findings,
        scores,
        summary,
        application_count,
    }))
}
