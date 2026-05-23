use std::sync::Arc;

use ambientor_k8s::K8sClient;
use ambientor_plan::{build_export_yaml, plan_to_rollout};
use ambientor_types::{MigrationPlan, PolicyTranslation};
use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use kube::{Api, api::ListParams};
use serde::Serialize;

use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanListItem {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub approved: bool,
    pub wave_count: i32,
    pub assessment_ref: Option<String>,
    pub waves: Vec<ambientor_types::MigrationWave>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationSummary {
    pub name: String,
    pub phase: String,
    pub source_name: String,
    pub suggested_manifest: Option<String>,
    pub warnings: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanDetail {
    #[serde(flatten)]
    pub plan: PlanListItem,
    pub translations: Vec<TranslationSummary>,
}

pub async fn list_plans(
    State(_state): State<Arc<AppState>>,
) -> Result<Json<Vec<PlanListItem>>, (StatusCode, String)> {
    let k8s = k8s_client().await?;
    let api: Api<MigrationPlan> = Api::all(k8s.client);
    let list = api.list(&ListParams::default()).await.map_err(internal)?;

    let mut items: Vec<PlanListItem> = list
        .items
        .into_iter()
        .filter_map(|p| plan_to_list_item(&p))
        .collect();
    items.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    Ok(Json(items))
}

pub async fn get_plan(
    State(_state): State<Arc<AppState>>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<PlanDetail>, (StatusCode, String)> {
    let k8s = k8s_client().await?;
    let plan = fetch_plan(&k8s, &namespace, &name).await?;
    let plan_item =
        plan_to_list_item(&plan).ok_or((StatusCode::NOT_FOUND, "plan has no status yet".into()))?;
    let translations = list_translations_in_namespace(&k8s, &namespace).await?;
    Ok(Json(PlanDetail {
        plan: plan_item,
        translations,
    }))
}

pub async fn export_plan(
    State(_state): State<Arc<AppState>>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Response, (StatusCode, String)> {
    let k8s = k8s_client().await?;
    let plan = fetch_plan(&k8s, &namespace, &name).await?;
    let pt_api: Api<PolicyTranslation> = Api::namespaced(k8s.client.clone(), &namespace);
    let pt_list = pt_api
        .list(&ListParams::default())
        .await
        .map_err(internal)?;
    let rollout = plan_to_rollout(&plan.spec);
    let yaml = build_export_yaml(&plan, &pt_list.items, &rollout).map_err(internal)?;
    Ok(([(header::CONTENT_TYPE, "application/x-yaml")], yaml).into_response())
}

async fn k8s_client() -> Result<K8sClient, (StatusCode, String)> {
    K8sClient::in_cluster()
        .await
        .or(K8sClient::from_kubeconfig(None).await)
        .map_err(internal)
}

async fn fetch_plan(
    k8s: &K8sClient,
    namespace: &str,
    name: &str,
) -> Result<MigrationPlan, (StatusCode, String)> {
    let api: Api<MigrationPlan> = Api::namespaced(k8s.client.clone(), namespace);
    api.get(name).await.map_err(|e| match e {
        kube::Error::Api(err) if err.code == 404 => (StatusCode::NOT_FOUND, err.to_string()),
        other => internal(other),
    })
}

async fn list_translations_in_namespace(
    k8s: &K8sClient,
    namespace: &str,
) -> Result<Vec<TranslationSummary>, (StatusCode, String)> {
    let api: Api<PolicyTranslation> = Api::namespaced(k8s.client.clone(), namespace);
    let list = api.list(&ListParams::default()).await.map_err(internal)?;
    let mut items: Vec<TranslationSummary> = list
        .items
        .into_iter()
        .filter_map(translation_to_summary)
        .collect();
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

fn plan_to_list_item(plan: &MigrationPlan) -> Option<PlanListItem> {
    let name = plan.metadata.name.clone()?;
    let namespace = plan
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());
    let status = plan.status.as_ref()?;
    Some(PlanListItem {
        name,
        namespace,
        phase: status.phase.clone(),
        approved: status.approved,
        wave_count: status.wave_count,
        assessment_ref: plan.spec.assessment_ref.clone(),
        waves: plan.spec.waves.clone(),
    })
}

fn translation_to_summary(pt: PolicyTranslation) -> Option<TranslationSummary> {
    let name = pt.metadata.name?;
    let status = pt.status?;
    Some(TranslationSummary {
        name,
        phase: status.phase,
        source_name: pt.spec.source_name,
        suggested_manifest: status.suggested_manifest,
        warnings: status.warnings,
    })
}

fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
