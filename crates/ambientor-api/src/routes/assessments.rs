use std::sync::Arc;

use ambientor_k8s::K8sClient;
use ambientor_types::{AmbientAssessment, FindingSummary};
use axum::{Json, extract::State};
use kube::{Api, api::ListParams};
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessmentListItem {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub scores: ambientor_types::AssessmentScores,
    pub summary: FindingSummary,
    pub findings: Vec<ambientor_types::Finding>,
}

pub async fn list_assessments(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<Vec<AssessmentListItem>>, (axum::http::StatusCode, String)> {
    let k8s = K8sClient::in_cluster()
        .await
        .or(K8sClient::from_kubeconfig(None).await)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let api: Api<AmbientAssessment> = Api::all(k8s.client);
    let list = api
        .list(&ListParams::default())
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut items: Vec<AssessmentListItem> = list
        .items
        .into_iter()
        .filter_map(|a| assessment_to_item(&a))
        .collect();
    items.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    Ok(Json(items))
}

fn assessment_to_item(a: &AmbientAssessment) -> Option<AssessmentListItem> {
    let name = a.metadata.name.clone()?;
    let namespace = a
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());
    let status = a.status.as_ref()?;
    Some(AssessmentListItem {
        name,
        namespace,
        phase: status.phase.clone(),
        scores: ambientor_types::AssessmentScores {
            readiness: status.readiness_score,
            sidecar_dependency: status.sidecar_dependency_score,
            traffic_compatibility: status.traffic_compatibility_score,
            overall: status.overall_score,
        },
        summary: status.summary.clone().unwrap_or_default(),
        findings: status.findings.clone(),
    })
}
