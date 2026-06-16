use std::sync::Arc;

use ambientor_db::{ApplicationListQuery, cluster_ref_from_env};
use ambientor_k8s::K8sClient;
use ambientor_mesh::mesh_instances::{discover_mesh_instances, resolve_mesh_target};
use ambientor_plan::{build_export_yaml, build_plan_from_selection, plan_to_rollout};
use ambientor_types::{MigrationPlan, PolicyTranslation};
use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use kube::{
    Api,
    api::{Patch, PatchParams, ListParams},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;

use super::rollouts::{
    approve_rollout_stage, ensure_rollout_for_plan, fetch_rollout, rollout_name_for_plan,
    rollout_to_list_item,
};
use super::dashboard::refresh_and_notify;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanListItem {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub approved: bool,
    pub wave_count: i32,
    pub assessment_ref: Option<String>,
    pub selected_namespaces: Vec<String>,
    pub cluster_ref: Option<String>,
    pub display_name: Option<String>,
    pub mesh_target: Option<ambientor_types::MeshTarget>,
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
    pub sync: PlanSyncStatus,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanSyncStatus {
    pub rollout_name: Option<String>,
    pub rollout_phase: Option<String>,
    pub rollout_awaiting_approval: bool,
    /// `execute` | `approve_rollout` | `running` | `completed` | `wait_plan` | `failed`
    pub next_action: String,
    pub channels: PlanChannelHints,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanChannelHints {
    pub portal: String,
    pub cli: String,
    pub gitops_plan_patch: String,
    pub gitops_rollout_patch: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanActorRequest {
    pub actor: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecutePlanResponse {
    pub plan_namespace: String,
    pub plan_name: String,
    pub plan_approved: bool,
    pub rollout_namespace: String,
    pub rollout_name: String,
    pub rollout_phase: String,
    pub message: String,
    pub sync: PlanSyncStatus,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMigrationPlanRequest {
    /// CR name (DNS-1123). Auto-generated when omitted.
    pub name: Option<String>,
    /// Namespace for the MigrationPlan CR (defaults to ambientor install ns).
    pub namespace: Option<String>,
    pub cluster_ref: Option<String>,
    pub display_name: Option<String>,
    pub assessment_ref: Option<String>,
    pub mesh_target: Option<ambientor_types::MeshTarget>,
    pub selected_namespaces: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateMigrationPlanResponse {
    pub name: String,
    pub namespace: String,
    pub phase: String,
    pub selected_count: usize,
    pub wave_count: usize,
    pub mesh_target: Option<ambientor_types::MeshTarget>,
    /// Labels applied during rollout (enrollment + ambient dataplane).
    pub namespace_labels_preview: Vec<NamespaceLabelPreview>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceLabelPreview {
    pub key: String,
    pub value: String,
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

pub async fn create_plan(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateMigrationPlanRequest>,
) -> Result<Json<CreateMigrationPlanResponse>, (StatusCode, String)> {
    if body.selected_namespaces.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "selectedNamespaces must not be empty".into(),
        ));
    }

    let mut selected: Vec<String> = body
        .selected_namespaces
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    selected.sort();
    selected.dedup();
    if selected.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "selectedNamespaces must not be empty".into(),
        ));
    }

    if let Some(store) = state.applications_store() {
        let cluster_ref = body
            .cluster_ref
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(cluster_ref_from_env);
        let apps = store
            .list_applications(ApplicationListQuery {
                cluster_ref: cluster_ref.clone(),
                search: None,
                risk_level: None,
                mesh_revision: None,
                migration_candidates_only: false,
                page: 1,
                page_size: 100_000,
            })
            .await
            .map_err(internal)?;
        let blocked: Vec<String> = apps
            .items
            .iter()
            .filter(|a| selected.contains(&a.namespace) && a.blocker_count > 0)
            .map(|a| a.namespace.clone())
            .collect();
        if !blocked.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "cannot migrate namespaces with blockers until resolved: {}",
                    blocked.join(", ")
                ),
            ));
        }
        let not_candidates: Vec<String> = apps
            .items
            .iter()
            .filter(|a| selected.contains(&a.namespace) && !a.migration_candidate)
            .map(|a| a.namespace.clone())
            .collect();
        if !not_candidates.is_empty() {
            return Err((
                StatusCode::BAD_REQUEST,
                format!(
                    "namespaces are not sidecar migration candidates (already ambient or not enrolled): {}",
                    not_candidates.join(", ")
                ),
            ));
        }
    }

    let k8s = k8s_client().await?;
    let instances = discover_mesh_instances(&k8s.client)
        .await
        .map_err(internal)?;
    let mesh = resolve_mesh_target(&instances, body.mesh_target.as_ref())
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let mesh_target = ambientor_types::MeshTarget {
        revision: Some(mesh.revision.clone()),
        discovery_label: if mesh.discovery_label.is_empty() {
            None
        } else {
            Some(mesh.discovery_label.clone())
        },
        control_plane_namespace: Some(mesh.control_plane_namespace.clone()),
    };

    let cluster_ref = body
        .cluster_ref
        .filter(|s| !s.is_empty())
        .or_else(|| Some(cluster_ref_from_env()));

    let spec = build_plan_from_selection(
        &selected,
        Some(mesh_target.clone()),
        cluster_ref.clone(),
        body.display_name.clone(),
        body.assessment_ref.clone(),
        None,
    );

    let plan_ns = body
        .namespace
        .filter(|s| !s.is_empty())
        .unwrap_or_else(default_plan_namespace);
    let plan_name = body.name.filter(|s| !s.is_empty()).unwrap_or_else(|| {
        let slug = selected
            .iter()
            .take(3)
            .map(|s| s.chars().take(12).collect::<String>())
            .collect::<Vec<_>>()
            .join("-");
        format!("migrate-{}-{}", slug, chrono::Utc::now().format("%Y%m%d%H%M%S"))
    });

    let cr = MigrationPlan::new(&plan_name, spec);
    let api: Api<MigrationPlan> = Api::namespaced(k8s.client.clone(), &plan_ns);
    let pp = PatchParams::apply("ambientor.io").force();
    api.patch(&plan_name, &pp, &Patch::Apply(&cr))
        .await
        .map_err(internal)?;

    let labels_preview = enrollment_label_preview(&mesh);

    Ok(Json(CreateMigrationPlanResponse {
        name: plan_name,
        namespace: plan_ns,
        phase: "Pending".into(),
        selected_count: selected.len(),
        wave_count: cr.spec.waves.len(),
        mesh_target: Some(mesh_target),
        namespace_labels_preview: labels_preview,
    }))
}

fn default_plan_namespace() -> String {
    std::env::var("AMBIENTOR_NAMESPACE").unwrap_or_else(|_| "ambientor-system".into())
}

fn enrollment_label_preview(mesh: &ambientor_types::MeshInstance) -> Vec<NamespaceLabelPreview> {
    let mut out = Vec::new();
    out.push(NamespaceLabelPreview {
        key: "istio.io/rev".into(),
        value: mesh
            .enrollment
            .revision_tag
            .clone()
            .unwrap_or_else(|| mesh.enrollment.revision.clone()),
    });
    if let (Some(k), Some(v)) = (
        mesh.enrollment.discovery_label_key.as_ref(),
        mesh.enrollment.discovery_label_value.as_ref(),
    ) {
        out.push(NamespaceLabelPreview {
            key: k.clone(),
            value: v.clone(),
        });
    }
    out.push(NamespaceLabelPreview {
        key: "istio.io/dataplane-mode".into(),
        value: "ambient".into(),
    });
    out
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
    let sync = build_plan_sync(&k8s, &namespace, &name, &plan_item).await;
    Ok(Json(PlanDetail {
        plan: plan_item,
        translations,
        sync,
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

/// Record human approval on the plan (P2). Same field used by GitOps `kubectl patch`.
pub async fn approve_plan(
    State(state): State<Arc<AppState>>,
    Path((namespace, name)): Path<(String, String)>,
    Json(body): Json<PlanActorRequest>,
) -> Result<Json<PlanListItem>, (StatusCode, String)> {
    let k8s = k8s_client().await?;
    let plan = fetch_plan(&k8s, &namespace, &name).await?;
    let status = plan
        .status
        .as_ref()
        .ok_or((StatusCode::CONFLICT, "plan has no status yet".into()))?;
    if status.phase != "Ready" {
        return Err((
            StatusCode::CONFLICT,
            format!("plan phase is {}; must be Ready", status.phase),
        ));
    }
    if !status.approved {
        patch_plan_approved(&k8s, &namespace, &name).await?;
        let actor = body.actor.as_deref().unwrap_or("api");
        if let Some(repo) = state.audit_store() {
            let event = audit_plan_approve(&namespace, &name, actor);
            if let Err(e) = repo.append(&event).await {
                tracing::warn!(error = %e, plan = %name, "failed to append plan approve audit");
            }
        }
    }
    let updated = fetch_plan(&k8s, &namespace, &name).await?;
    plan_to_list_item(&updated).ok_or((StatusCode::NOT_FOUND, "plan has no status".into()))
        .map(Json)
}

/// One-time approve: mark plan approved, ensure rollout exists, approve stage 0.
pub async fn execute_plan(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path((namespace, name)): Path<(String, String)>,
    Json(body): Json<PlanActorRequest>,
) -> Result<Json<ExecutePlanResponse>, (StatusCode, String)> {
    let jwt_actor = if state.auth.is_some() {
        let rollout_name = rollout_name_for_plan(&name);
        let claims = crate::authz::require_rollout_approve(
            &state,
            &headers,
            &namespace,
            &rollout_name,
        )
        .await?;
        Some(claims.username)
    } else {
        None
    };
    let actor = body
        .actor
        .or(jwt_actor)
        .unwrap_or_else(|| "portal".into());

    let k8s = k8s_client().await?;
    let plan = fetch_plan(&k8s, &namespace, &name).await?;
    let status = plan
        .status
        .as_ref()
        .ok_or((StatusCode::CONFLICT, "plan has no status yet".into()))?;
    if status.phase != "Ready" {
        return Err((
            StatusCode::CONFLICT,
            format!("plan phase is {}; wait until Ready", status.phase),
        ));
    }

    if !status.approved {
        patch_plan_approved(&k8s, &namespace, &name).await?;
        if let Some(repo) = state.audit_store() {
            let event = audit_plan_approve(&namespace, &name, &actor);
            if let Err(e) = repo.append(&event).await {
                tracing::warn!(error = %e, plan = %name, "failed to append plan approve audit");
            }
        }
    }

    let rollout_name = ensure_rollout_for_plan(&k8s, &plan).await?;
    let rollout = fetch_rollout(&k8s, &namespace, &rollout_name).await?;
    let rollout_status = rollout
        .status
        .as_ref()
        .ok_or((StatusCode::CONFLICT, "rollout has no status yet".into()))?;

    let rollout_phase = if rollout_status.phase == "AwaitingApproval" {
        approve_rollout_stage(
            &state,
            &k8s,
            &namespace,
            &rollout_name,
            Some(rollout_status.current_stage),
            &actor,
        )
        .await?;
        fetch_rollout(&k8s, &namespace, &rollout_name)
            .await?
            .status
            .as_ref()
            .map(|s| s.phase.clone())
            .unwrap_or_else(|| "Running".into())
    } else {
        rollout_status.phase.clone()
    };

    let plan_item = plan_to_list_item(&fetch_plan(&k8s, &namespace, &name).await?)
        .ok_or((StatusCode::NOT_FOUND, "plan has no status".into()))?;
    let sync = build_plan_sync(&k8s, &namespace, &name, &plan_item).await;
    let cluster_ref = cluster_ref_from_env();
    refresh_and_notify(&state, &cluster_ref).await;

    Ok(Json(ExecutePlanResponse {
        plan_namespace: namespace.clone(),
        plan_name: name.clone(),
        plan_approved: true,
        rollout_namespace: namespace,
        rollout_name: rollout_name.clone(),
        rollout_phase: rollout_phase.clone(),
        message: "Plan approved and migration pipeline started (one approval; remaining stages run automatically)".into(),
        sync,
    }))
}

pub(super) async fn k8s_client() -> Result<K8sClient, (StatusCode, String)> {
    K8sClient::in_cluster()
        .await
        .or(K8sClient::from_kubeconfig(None).await)
        .map_err(internal)
}

pub(super) async fn fetch_plan(
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
        selected_namespaces: plan.spec.selected_namespaces.clone(),
        cluster_ref: plan.spec.cluster_ref.clone().or(status.cluster_ref.clone()),
        display_name: plan.spec.display_name.clone(),
        mesh_target: plan.spec.mesh_target.clone(),
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

pub(super) fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

async fn patch_plan_approved(
    k8s: &K8sClient,
    namespace: &str,
    name: &str,
) -> Result<(), (StatusCode, String)> {
    let api: Api<MigrationPlan> = Api::namespaced(k8s.client.clone(), namespace);
    let patch = serde_json::json!({ "status": { "approved": true } });
    api.patch_status(name, &Default::default(), &Patch::Merge(&patch))
        .await
        .map_err(internal)?;
    Ok(())
}

fn audit_plan_approve(namespace: &str, name: &str, actor: &str) -> ambientor_types::AuditEvent {
    ambientor_types::AuditEvent {
        id: Uuid::new_v4(),
        timestamp: Utc::now(),
        actor: actor.to_string(),
        action: "plan.approve".into(),
        resource: format!("migrationplan/{namespace}/{name}"),
        outcome: "succeeded".into(),
        details: None,
    }
}

async fn build_plan_sync(
    k8s: &K8sClient,
    namespace: &str,
    plan_name: &str,
    plan: &PlanListItem,
) -> PlanSyncStatus {
    let rollout_name = rollout_name_for_plan(plan_name);
    let rollout = fetch_rollout(k8s, namespace, &rollout_name).await.ok();
    let rollout_item = rollout.as_ref().and_then(rollout_to_list_item);
    let rollout_phase = rollout_item.as_ref().map(|r| r.phase.clone());
    let rollout_awaiting = rollout_item
        .as_ref()
        .is_some_and(|r| r.awaiting_approval);

    let next_action = if plan.phase != "Ready" {
        "wait_plan".into()
    } else if rollout_item.as_ref().is_some_and(|r| r.phase == "Completed") {
        "completed".into()
    } else if rollout_item.as_ref().is_some_and(|r| r.phase == "Failed") {
        "failed".into()
    } else if rollout_item.as_ref().is_some_and(|r| {
        r.phase == "Running" || r.phase == "Pending"
    }) {
        "running".into()
    } else if rollout_awaiting {
        "approve_rollout".into()
    } else if !plan.approved {
        "execute".into()
    } else {
        "execute".into()
    };

    let gitops_plan_patch = format!(
        "kubectl patch migrationplan {plan_name} -n {namespace} --type=merge -p '{{\"status\":{{\"approved\":true}}}}'"
    );
    let gitops_rollout_patch = format!(
        "kubectl patch rollout {rollout_name} -n {namespace} --type=merge -p '{{\"status\":{{\"approvedStage\":0,\"phase\":\"Pending\"}}}}'"
    );
    let cli = format!("ambientor plan execute -n {namespace} {plan_name}");

    PlanSyncStatus {
        rollout_name: rollout_item.as_ref().map(|_| rollout_name),
        rollout_phase,
        rollout_awaiting_approval: rollout_awaiting,
        next_action,
        channels: PlanChannelHints {
            portal: "Plans → Approve & run migration (one click)".into(),
            cli,
            gitops_plan_patch,
            gitops_rollout_patch,
        },
    }
}
