use std::sync::Arc;

use ambientor_k8s::K8sClient;
use ambientor_mesh::{
    OpenShiftWizardOptions, OpenShiftWizardReport, namespaces_needing_enrollment, run_wizard,
};
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct WizardQuery {
    /// Comma-separated namespaces to include in MemberRoll suggestion.
    #[serde(default)]
    pub enroll: Option<String>,
    /// Ambientor install namespace for SCC check.
    #[serde(default = "default_ambientor_ns")]
    pub ambientor_namespace: String,
    #[serde(default = "default_operator_sa")]
    pub operator_service_account: String,
}

fn default_ambientor_ns() -> String {
    "ambientor-system".into()
}

fn default_operator_sa() -> String {
    "ambientor-operator".into()
}

pub async fn openshift_wizard(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<WizardQuery>,
) -> Result<Json<OpenShiftWizardReport>, (StatusCode, String)> {
    let k8s = K8sClient::in_cluster()
        .await
        .or(K8sClient::from_kubeconfig(None).await)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let platform = ambientor_k8s::detect_platform(&k8s.client)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut enroll: Vec<String> = query
        .enroll
        .as_deref()
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();

    if enroll.is_empty() {
        enroll = namespaces_needing_enrollment(&k8s.client)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    let opts = OpenShiftWizardOptions {
        ambientor_namespace: query.ambientor_namespace,
        operator_service_account: query.operator_service_account,
        enroll_namespaces: enroll,
    };

    let report = run_wizard(&k8s.client, &platform, &opts)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(report))
}
