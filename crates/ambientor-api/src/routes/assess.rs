use std::sync::Arc;

use ambientor_core::scoring::compute_scores;
use ambientor_db::{ApplicationListQuery, StoredAssessment, cluster_ref_from_env};
use ambientor_k8s::K8sClient;
use ambientor_mesh::backend::backend_for_flavor;
use ambientor_scan::default_registry;
use ambientor_types::FindingSummary;
use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::routes::applications::persist_assessment_from_findings;
use crate::routes::assessment_crd::{direct_assess_enabled, trigger_and_wait};
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
    /// `crd` when triggered via `AmbientAssessment`; `direct` for inline API scan.
    pub trigger: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessment_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assessment_namespace: Option<String>,
}

pub async fn assess(
    State(state): State<Arc<AppState>>,
    Json(body): Json<AssessRequest>,
) -> Result<Json<AssessResponse>, (axum::http::StatusCode, String)> {
    let cluster_ref = cluster_ref_from_env();

    if direct_assess_enabled() {
        return assess_direct(state, body, &cluster_ref).await;
    }

    let k8s = k8s_client().await?;
    match trigger_and_wait(&k8s.client, &cluster_ref).await {
        Ok(completed) => {
            let findings = completed.status.findings.clone();
            let scores = completed.scores();
            let summary = completed.summary();

            publish_completed(&state, &cluster_ref, findings.len()).await;

            let application_count = application_count_for_cluster(&state, &cluster_ref).await;

            Ok(Json(AssessResponse {
                findings,
                scores,
                summary,
                application_count,
                trigger: "crd".into(),
                assessment_name: Some(completed.name),
                assessment_namespace: Some(completed.namespace),
            }))
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "AmbientAssessment trigger failed; falling back to direct assessment"
            );
            assess_direct(state, body, &cluster_ref).await
        }
    }
}

async fn assess_direct(
    state: Arc<AppState>,
    body: AssessRequest,
    cluster_ref: &str,
) -> Result<Json<AssessResponse>, (axum::http::StatusCode, String)> {
    let namespace_filter = body.namespace.as_deref();
    let k8s = k8s_client().await?;

    let platform = ambientor_k8s::detect_platform(&k8s.client)
        .await
        .map_err(internal)?;

    let backend = backend_for_flavor(platform.mesh_flavor);
    let mut ctx = backend
        .build_rule_context(&k8s.client)
        .await
        .map_err(internal)?;

    if let Ok(Some(ver)) = backend.detect_version(&k8s.client).await {
        ctx.mesh_version = Some(ver);
    }

    let registry = default_registry();
    let findings = registry.evaluate_all(&ctx);
    let scores = compute_scores(&findings);
    let summary = FindingSummary::from_findings(&findings);

    publish_completed(&state, cluster_ref, findings.len()).await;

    if let Some(repo) = state.scan_store() {
        let payload = StoredAssessment {
            findings: findings.clone(),
            scores: scores.clone(),
            summary: summary.clone(),
            source: Some("api".into()),
            assessment_name: None,
        };
        if let Err(e) = repo
            .record_completed(cluster_ref, namespace_filter, &payload)
            .await
        {
            tracing::warn!(error = %e, "failed to persist scan run");
        }
    }

    let application_count = persist_assessment_from_findings(
        state.as_ref(),
        &k8s.client,
        cluster_ref,
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
        trigger: "direct".into(),
        assessment_name: None,
        assessment_namespace: None,
    }))
}

async fn publish_completed(state: &AppState, cluster_ref: &str, finding_count: usize) {
    state.sse.write().await.publish(
        "assessment",
        &serde_json::json!({
            "phase": "completed",
            "findingCount": finding_count,
            "clusterRef": cluster_ref,
        }),
    );
}

async fn application_count_for_cluster(state: &AppState, cluster_ref: &str) -> usize {
    let Some(store) = state.applications_store() else {
        return 0;
    };
    store
        .list_applications(ApplicationListQuery {
            cluster_ref: cluster_ref.to_string(),
            ..ApplicationListQuery::default()
        })
        .await
        .map(|p| p.total as usize)
        .unwrap_or(0)
}

async fn k8s_client() -> Result<K8sClient, (axum::http::StatusCode, String)> {
    K8sClient::in_cluster()
        .await
        .or(K8sClient::from_kubeconfig(None).await)
        .map_err(internal)
}

fn internal(e: impl std::fmt::Display) -> (axum::http::StatusCode, String) {
    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
