use std::sync::Arc;

use ambientor_k8s::K8sClient;
use ambientor_plan::plan_to_rollout;
use ambientor_rollout::audit::audit_rollout_approve;
use ambientor_types::{MeshInstance, Rollout, RolloutStage, RolloutStageType, RolloutStatus};
use ambientor_db::cluster_ref_from_env;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use kube::{
    Api,
    api::{ListParams, Patch, PatchParams},
};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

use super::plans::{fetch_plan, internal, k8s_client};

#[derive(Debug, Deserialize)]
pub struct RolloutsQuery {
    #[serde(rename = "clusterRef")]
    pub cluster_ref: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RolloutListItem {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub current_stage: i32,
    pub approved_stage: i32,
    pub stage_count: usize,
    pub plan_ref: Option<String>,
    pub cluster_ref: Option<String>,
    pub awaiting_approval: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RolloutStageView {
    pub index: i32,
    pub name: String,
    pub stage_type: String,
    pub namespaces: Vec<String>,
    pub requires_approval: bool,
    pub result_phase: Option<String>,
    pub result_message: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RolloutConditionView {
    pub r#type: String,
    pub status: String,
    pub reason: Option<String>,
    pub message: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RolloutDetail {
    #[serde(flatten)]
    pub rollout: RolloutListItem,
    pub stages: Vec<RolloutStageView>,
    pub auto_rollback: bool,
    pub resolved_mesh_target: Option<MeshInstance>,
    pub conditions: Vec<RolloutConditionView>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveRolloutRequest {
    /// Stage index to approve; defaults to `currentStage` when omitted.
    pub stage: Option<i32>,
    /// Actor recorded in the audit log (defaults to `api`).
    pub actor: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveRolloutResponse {
    pub name: String,
    pub namespace: String,
    pub approved_stage: i32,
    pub current_stage: i32,
    pub phase: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRolloutResponse {
    pub name: String,
    pub namespace: String,
}

pub async fn list_rollouts(
    State(_state): State<Arc<AppState>>,
    Query(query): Query<RolloutsQuery>,
) -> Result<Json<Vec<RolloutListItem>>, (StatusCode, String)> {
    let k8s = k8s_client().await?;
    let filter_cluster = query
        .cluster_ref
        .filter(|s| !s.is_empty())
        .unwrap_or_else(cluster_ref_from_env);
    let api: Api<Rollout> = Api::all(k8s.client);
    let list = api.list(&ListParams::default()).await.map_err(internal)?;
    let mut items: Vec<RolloutListItem> = list
        .items
        .into_iter()
        .filter_map(|r| rollout_to_list_item(&r))
        .filter(|item| rollout_matches_cluster(item, &filter_cluster))
        .collect();
    items.sort_by(|a, b| a.namespace.cmp(&b.namespace).then(a.name.cmp(&b.name)));
    Ok(Json(items))
}

pub async fn get_rollout(
    State(_state): State<Arc<AppState>>,
    Path((namespace, name)): Path<(String, String)>,
) -> Result<Json<RolloutDetail>, (StatusCode, String)> {
    let k8s = k8s_client().await?;
    let rollout = fetch_rollout(&k8s, &namespace, &name).await?;
    let item = rollout_to_list_item(&rollout)
        .ok_or((StatusCode::NOT_FOUND, "rollout has no status yet".into()))?;
    let status = rollout.status.as_ref();
    Ok(Json(RolloutDetail {
        stages: stages_with_results(&rollout.spec.stages, status),
        auto_rollback: rollout.spec.auto_rollback,
        resolved_mesh_target: status.and_then(|s| s.resolved_mesh_target.clone()),
        conditions: status
            .map(|s| {
                s.conditions
                    .iter()
                    .map(|c| RolloutConditionView {
                        r#type: c.r#type.clone(),
                        status: c.status.clone(),
                        reason: c.reason.clone(),
                        message: c.message.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        rollout: item,
    }))
}

pub async fn approve_rollout(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((namespace, name)): Path<(String, String)>,
    Json(body): Json<ApproveRolloutRequest>,
) -> Result<Json<ApproveRolloutResponse>, (StatusCode, String)> {
    let jwt_actor = if state.auth.is_some() {
        let claims =
            crate::authz::require_rollout_approve(&state, &headers, &namespace, &name).await?;
        Some(claims.username)
    } else {
        None
    };
    let k8s = k8s_client().await?;
    let rollout = fetch_rollout(&k8s, &namespace, &name).await?;
    let status = rollout
        .status
        .as_ref()
        .ok_or((StatusCode::CONFLICT, "rollout has no status yet".into()))?;

    let stage_to_approve = body.stage.unwrap_or(status.current_stage);
    validate_approval(status, stage_to_approve, rollout.spec.stages.len())?;

    let api: Api<Rollout> = Api::namespaced(k8s.client.clone(), &namespace);
    let phase = if status.phase == "RolledBack" {
        "Pending".to_string()
    } else {
        status.phase.clone()
    };
    let patch = serde_json::json!({
        "status": {
            "approvedStage": stage_to_approve,
            "phase": phase,
        }
    });
    api.patch_status(&name, &Default::default(), &Patch::Merge(&patch))
        .await
        .map_err(internal)?;

    let actor = body.actor.or(jwt_actor).unwrap_or_else(|| "api".into());
    if let Some(repo) = state.audit_store() {
        let event = audit_rollout_approve(&namespace, &name, &actor, stage_to_approve);
        if let Err(e) = repo.append(&event).await {
            tracing::warn!(error = %e, rollout = %name, "failed to append rollout approve audit");
        }
    }

    let updated = fetch_rollout(&k8s, &namespace, &name).await?;
    let new_status = updated.status.as_ref().unwrap();

    Ok(Json(ApproveRolloutResponse {
        name,
        namespace,
        approved_stage: new_status.approved_stage,
        current_stage: new_status.current_stage,
        phase: new_status.phase.clone(),
    }))
}

/// Create a `Rollout` CR from an existing `MigrationPlan` (`{plan-name}-rollout`).
pub async fn create_rollout_from_plan(
    State(_state): State<Arc<AppState>>,
    Path((namespace, plan_name)): Path<(String, String)>,
) -> Result<Json<CreateRolloutResponse>, (StatusCode, String)> {
    let k8s = k8s_client().await?;
    let plan = fetch_plan(&k8s, &namespace, &plan_name).await?;
    let rollout_name = rollout_name_for_plan(&plan_name);

    let existing: Api<Rollout> = Api::namespaced(k8s.client.clone(), &namespace);
    if existing.get(&rollout_name).await.is_ok() {
        return Err((
            StatusCode::CONFLICT,
            format!("Rollout {namespace}/{rollout_name} already exists"),
        ));
    }

    let mut spec = plan_to_rollout(&plan.spec);
    spec.plan_ref = Some(plan_name.clone());
    spec.mesh_target = plan.spec.mesh_target.clone();
    let cr = Rollout::new(&rollout_name, spec);
    let pp = PatchParams::apply("ambientor.io").force();
    existing
        .patch(&rollout_name, &pp, &Patch::Apply(&cr))
        .await
        .map_err(internal)?;

    let status_patch = serde_json::json!({
        "status": {
            "phase": "AwaitingApproval",
            "currentStage": 0,
            "approvedStage": -1,
            "stageResults": []
        }
    });
    existing
        .patch_status(&rollout_name, &Default::default(), &Patch::Merge(&status_patch))
        .await
        .map_err(internal)?;

    Ok(Json(CreateRolloutResponse {
        name: rollout_name,
        namespace,
    }))
}

pub fn rollout_name_for_plan(plan_name: &str) -> String {
    format!("{plan_name}-rollout")
}

pub fn validate_approval(
    status: &RolloutStatus,
    stage_to_approve: i32,
    stage_count: usize,
) -> Result<(), (StatusCode, String)> {
    if stage_to_approve < 0 || stage_to_approve as usize >= stage_count {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "stage index {stage_to_approve} out of range (0..{})",
                stage_count
            ),
        ));
    }
    if stage_to_approve != status.current_stage {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "can only approve current stage {}; requested {stage_to_approve}",
                status.current_stage
            ),
        ));
    }
    if status.phase == "Completed" {
        return Err((StatusCode::CONFLICT, "rollout already completed".into()));
    }
    if status.phase == "Failed" {
        return Err((
            StatusCode::CONFLICT,
            "rollout failed; fix or delete before approving".into(),
        ));
    }
    if status.approved_stage >= stage_to_approve && status.phase != "AwaitingApproval" {
        return Err((
            StatusCode::CONFLICT,
            format!("stage {stage_to_approve} already approved"),
        ));
    }
    Ok(())
}

async fn fetch_rollout(
    k8s: &K8sClient,
    namespace: &str,
    name: &str,
) -> Result<Rollout, (StatusCode, String)> {
    let api: Api<Rollout> = Api::namespaced(k8s.client.clone(), namespace);
    api.get(name).await.map_err(|e| match e {
        kube::Error::Api(err) if err.code == 404 => (StatusCode::NOT_FOUND, err.to_string()),
        other => internal(other),
    })
}

fn rollout_to_list_item(rollout: &Rollout) -> Option<RolloutListItem> {
    let name = rollout.metadata.name.clone()?;
    let namespace = rollout
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());
    let status = rollout.status.as_ref()?;
    let awaiting_approval = status.phase == "AwaitingApproval";
    Some(RolloutListItem {
        name,
        namespace,
        phase: status.phase.clone(),
        current_stage: status.current_stage,
        approved_stage: status.approved_stage,
        stage_count: rollout.spec.stages.len(),
        plan_ref: rollout.spec.plan_ref.clone(),
        cluster_ref: rollout.spec.cluster_ref.clone(),
        awaiting_approval,
    })
}

fn rollout_matches_cluster(item: &RolloutListItem, cluster_ref: &str) -> bool {
    match &item.cluster_ref {
        Some(cr) => cr == cluster_ref,
        None => true,
    }
}

fn stage_type_label(t: &RolloutStageType) -> &'static str {
    match t {
        RolloutStageType::InstallAmbientComponents => "Install ambient components",
        RolloutStageType::EnrollNamespace => "Enroll namespace",
        RolloutStageType::DeployWaypoint => "Deploy waypoint",
        RolloutStageType::LabelNamespace => "Label namespace",
        RolloutStageType::TranslatePolicy => "Translate policy",
        RolloutStageType::RollingRestart => "Rolling restart",
        RolloutStageType::RemoveInjection => "Remove injection",
        RolloutStageType::VerifyTraffic => "Verify traffic",
        RolloutStageType::DryRun => "Dry run",
    }
}

fn stages_with_results(
    stages: &[RolloutStage],
    status: Option<&RolloutStatus>,
) -> Vec<RolloutStageView> {
    stages
        .iter()
        .enumerate()
        .map(|(idx, stage)| {
            let result = status.and_then(|s| s.stage_results.iter().find(|r| r.name == stage.name));
            RolloutStageView {
                index: idx as i32,
                name: stage.name.clone(),
                stage_type: stage_type_label(&stage.r#type).to_string(),
                namespaces: stage.namespaces.clone(),
                requires_approval: stage.requires_approval,
                result_phase: result.map(|r| r.phase.clone()),
                result_message: result.and_then(|r| r.message.clone()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn status(current: i32, approved: i32, phase: &str) -> RolloutStatus {
        RolloutStatus {
            phase: phase.into(),
            current_stage: current,
            approved_stage: approved,
            ..Default::default()
        }
    }

    #[test]
    fn approve_current_stage_ok() {
        assert!(validate_approval(&status(2, 1, "AwaitingApproval"), 2, 5).is_ok());
    }

    #[test]
    fn reject_wrong_stage() {
        assert!(validate_approval(&status(2, 1, "AwaitingApproval"), 3, 5).is_err());
    }
}
