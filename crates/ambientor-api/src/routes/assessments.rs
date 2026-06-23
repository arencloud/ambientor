use std::sync::Arc;

use ambientor_db::cluster_ref_from_env;
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
    State(state): State<Arc<AppState>>,
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

    let mut items: Vec<AssessmentListItem> = Vec::new();
    for a in list.items {
        if let Some(mut item) = assessment_to_item(&a) {
            enrich_findings_from_store(&state, &a, &mut item).await;
            items.push(item);
        }
    }
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

async fn enrich_findings_from_store(
    state: &AppState,
    assessment: &AmbientAssessment,
    item: &mut AssessmentListItem,
) {
    if !item.findings.is_empty() {
        return;
    }
    let Some(repo) = state.scan_store() else {
        return;
    };
    let cluster_ref = assessment
        .spec
        .cluster_ref
        .clone()
        .unwrap_or_else(cluster_ref_from_env);
    let Ok(Some(stored)) = repo.latest_for_assessment(&cluster_ref, &item.name).await else {
        return;
    };
    item.findings = stored.findings;
    if item.summary.blockers == 0 && item.summary.warnings == 0 && item.summary.info == 0 {
        item.summary = stored.summary;
    }
}
