use std::sync::Arc;

use ambientor_plan::{
    assessment_result_from_status_and_findings, build_plan, build_plan_from_selection,
    namespaces_for_planning, namespaces_matching_selector, namespaces_with_blockers,
    plan_name_for_assessment,
};
use ambientor_db::{ScanStore, cluster_ref_from_env};
use ambientor_k8s::client_for_cluster_ref;
use ambientor_types::{AmbientAssessment, AmbientAssessmentStatus, MeshInventory, MigrationPlan, MigrationPlanSpec};
use futures::StreamExt;
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
    runtime::controller::{Action, Controller},
    runtime::watcher::Config,
};
use tracing::info;

use super::inventory::FIELD_MANAGER;
use super::policy_translation::ensure_translations_in_namespace;
use super::runtime::{ReconcileError, ReconcileResult, error_policy};

pub async fn run(client: Client, scan_repo: Option<Arc<dyn ScanStore>>) {
    let ctx = Arc::new(PlanContext { client, scan_repo });
    Controller::new(Api::<MigrationPlan>::all(ctx.client.clone()), Config::default())
        .shutdown_on_signal()
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(e) = res {
                tracing::error!(error = ?e, "migrationplan controller error");
            }
        })
        .await;
}

struct PlanContext {
    client: Client,
    scan_repo: Option<Arc<dyn ScanStore>>,
}

async fn reconcile(plan: Arc<MigrationPlan>, ctx: Arc<PlanContext>) -> ReconcileResult {
    if plan.status.as_ref().is_some_and(|s| s.approved) {
        return Ok(Action::await_change());
    }
    reconcile_inner(&ctx, &plan)
        .await
        .map_err(ReconcileError::Other)?;
    Ok(Action::await_change())
}

async fn reconcile_inner(ctx: &PlanContext, plan: &MigrationPlan) -> anyhow::Result<()> {
    let work_client = client_for_cluster_ref(&ctx.client, plan.spec.cluster_ref.as_deref())
        .await
        .map_err(|e| anyhow::anyhow!("plan target cluster client: {e}"))?;
    let client = &ctx.client;
    let ns = plan
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());
    let plan_name = plan
        .metadata
        .name
        .clone()
        .ok_or_else(|| anyhow::anyhow!("MigrationPlan missing metadata.name"))?;

    let selected = &plan.spec.selected_namespaces;
    let has_selection = !selected.is_empty();
    let has_waves = !plan.spec.waves.is_empty();

    // User-supplied waves (CRD/gitops) — mark ready without re-planning.
    if has_waves && has_selection {
        let wave_count = plan.spec.waves.len() as i32;
        patch_plan_status(
            client,
            &ns,
            &plan_name,
            "Ready",
            wave_count,
            Some(selected.len() as i32),
            plan.spec.cluster_ref.clone(),
        )
        .await?;
        ensure_translations_for_plan(&work_client, plan).await;
        return Ok(());
    }

    let spec = if has_selection {
        reconcile_selection_plan(ctx, plan, &ns).await?
    } else if plan.spec.assessment_ref.is_some() {
        reconcile_assessment_plan(ctx, plan, &ns, &work_client).await?
    } else if has_waves {
        return Ok(());
    } else {
        patch_plan_status(
            client,
            &ns,
            &plan_name,
            "Pending",
            0,
            None,
            plan.spec.cluster_ref.clone(),
        )
        .await?;
        return Ok(());
    };

    let plan_api: Api<MigrationPlan> = Api::namespaced(client.clone(), &ns);
    let updated = MigrationPlan::new(&plan_name, spec);
    let pp = PatchParams::apply(FIELD_MANAGER).force();
    plan_api
        .patch(&plan_name, &pp, &Patch::Apply(&updated))
        .await?;

    let plan = plan_api.get(&plan_name).await?;
    let wave_count = plan.spec.waves.len() as i32;
    let selected_count = if plan.spec.selected_namespaces.is_empty() {
        None
    } else {
        Some(plan.spec.selected_namespaces.len() as i32)
    };
    patch_plan_status(
        client,
        &ns,
        &plan_name,
        "Ready",
        wave_count,
        selected_count,
        plan.spec.cluster_ref.clone(),
    )
    .await?;

    ensure_translations_for_plan(&work_client, &plan).await;

    info!(
        plan = %plan_name,
        waves = wave_count,
        selected = ?plan.spec.selected_namespaces.len(),
        "migration plan reconciled"
    );
    Ok(())
}

async fn reconcile_selection_plan(
    ctx: &PlanContext,
    plan: &MigrationPlan,
    assessment_namespace: &str,
) -> anyhow::Result<MigrationPlanSpec> {
    let client = &ctx.client;
    let assessment_result = optional_assessment_result(
        ctx,
        assessment_namespace,
        plan.spec.assessment_ref.as_deref(),
    )
    .await?;

    if let Some(ref ar) = assessment_result {
        let blocked = namespaces_with_blockers(&plan.spec.selected_namespaces, ar);
        if !blocked.is_empty() {
            let plan_name = plan.metadata.name.clone().unwrap_or_default();
            patch_plan_status(
                client,
                assessment_namespace,
                &plan_name,
                "Failed",
                0,
                Some(plan.spec.selected_namespaces.len() as i32),
                plan.spec.cluster_ref.clone(),
            )
            .await?;
            anyhow::bail!(
                "selected namespaces include blockers (not migratable until resolved): {}",
                blocked.join(", ")
            );
        }
    }

    Ok(build_plan_from_selection(
        &plan.spec.selected_namespaces,
        plan.spec.mesh_target.clone(),
        plan.spec.cluster_ref.clone(),
        plan.spec.display_name.clone(),
        plan.spec.assessment_ref.clone(),
        assessment_result.as_ref(),
    ))
}

async fn reconcile_assessment_plan(
    ctx: &PlanContext,
    plan: &MigrationPlan,
    assessment_namespace: &str,
    work_client: &Client,
) -> anyhow::Result<MigrationPlanSpec> {
    let client = &ctx.client;
    let plan_name = plan.metadata.name.clone().unwrap_or_default();
    let assessment_ref = plan
        .spec
        .assessment_ref
        .clone()
        .ok_or_else(|| anyhow::anyhow!("MigrationPlan {plan_name} missing spec.assessmentRef"))?;

    let assess_api: Api<AmbientAssessment> = Api::namespaced(client.clone(), assessment_namespace);
    let assessment = match assess_api.get(&assessment_ref).await {
        Ok(a) => a,
        Err(kube::Error::Api(e)) if e.code == 404 => {
            patch_plan_status(
                client,
                assessment_namespace,
                &plan_name,
                "Failed",
                0,
                None,
                plan.spec.cluster_ref.clone(),
            )
            .await?;
            return Ok(plan.spec.clone());
        }
        Err(e) => return Err(e.into()),
    };

    let status = assessment
        .status
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("assessment {assessment_ref} has no status"))?;

    if status.phase != "Completed" {
        patch_plan_status(
            client,
            assessment_namespace,
            &plan_name,
            "WaitingForAssessment",
            0,
            None,
            plan.spec.cluster_ref.clone(),
        )
        .await?;
        return Ok(plan.spec.clone());
    }

    let assessment_result =
        assessment_result_for_assessment(ctx, &assessment, status).await;
    let inventory_namespaces =
        inventory_target_namespaces(work_client, &assessment, assessment_namespace)
            .await
            .unwrap_or_default();
    let namespaces = namespaces_for_planning(&assessment_result.findings, &inventory_namespaces);
    let built = build_plan(&assessment_result, &namespaces);
    Ok(MigrationPlanSpec {
        assessment_ref: Some(assessment_ref),
        selected_namespaces: plan.spec.selected_namespaces.clone(),
        cluster_ref: plan.spec.cluster_ref.clone(),
        display_name: plan.spec.display_name.clone(),
        target_mesh_mode: built.target_mesh_mode,
        mesh_target: plan.spec.mesh_target.clone().or(built.mesh_target),
        ambient_ingress_gateway: plan.spec.ambient_ingress_gateway.clone(),
        waves: built.waves,
    })
}

async fn optional_assessment_result(
    ctx: &PlanContext,
    ns: &str,
    assessment_ref: Option<&str>,
) -> anyhow::Result<Option<ambientor_core::inventory::AssessmentResult>> {
    let client = &ctx.client;
    let Some(name) = assessment_ref else {
        return Ok(None);
    };
    let assess_api: Api<AmbientAssessment> = Api::namespaced(client.clone(), ns);
    let assessment = assess_api.get(name).await?;
    let status = assessment
        .status
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("assessment {name} has no status"))?;
    if status.phase != "Completed" {
        anyhow::bail!("assessment {name} is not Completed (phase={})", status.phase);
    }
    Ok(Some(
        assessment_result_for_assessment(ctx, &assessment, status).await,
    ))
}

async fn assessment_result_for_assessment(
    ctx: &PlanContext,
    assessment: &AmbientAssessment,
    status: &AmbientAssessmentStatus,
) -> ambientor_core::inventory::AssessmentResult {
    let stored_findings = if status.findings.is_empty() {
        if let (Some(repo), Some(name)) = (ctx.scan_repo.as_ref(), assessment.metadata.name.as_ref())
        {
            let cluster_ref = assessment
                .spec
                .cluster_ref
                .clone()
                .unwrap_or_else(cluster_ref_from_env);
            repo.latest_for_assessment(&cluster_ref, name)
                .await
                .ok()
                .flatten()
                .map(|stored| stored.findings)
        } else {
            None
        }
    } else {
        None
    };
    assessment_result_from_status_and_findings(status, stored_findings)
}

async fn ensure_translations_for_plan(client: &Client, plan: &MigrationPlan) {
    for wave in &plan.spec.waves {
        if wave.name == "wave-blocked" {
            continue;
        }
        for wave_ns in &wave.namespaces {
            if let Err(e) = ensure_translations_in_namespace(client, wave_ns).await {
                tracing::warn!(
                    error = %e,
                    namespace = %wave_ns,
                    "failed to ensure policy translations"
                );
            }
        }
    }
}

async fn inventory_target_namespaces(
    client: &Client,
    assessment: &AmbientAssessment,
    assessment_namespace: &str,
) -> anyhow::Result<Vec<String>> {
    let Some(inv_name) = assessment.spec.inventory_ref.as_ref() else {
        return Ok(Vec::new());
    };
    let inv_api: Api<MeshInventory> = Api::namespaced(client.clone(), assessment_namespace);
    let inv = inv_api.get(inv_name).await?;
    namespaces_matching_selector(client, &inv.spec.namespace_selector)
        .await
        .map_err(Into::into)
}

async fn patch_plan_status(
    client: &Client,
    ns: &str,
    name: &str,
    phase: &str,
    wave_count: i32,
    selected_count: Option<i32>,
    cluster_ref: Option<String>,
) -> anyhow::Result<()> {
    let api: Api<MigrationPlan> = Api::namespaced(client.clone(), ns);
    let mut status = serde_json::json!({
        "status": {
            "phase": phase,
            "approved": false,
            "waveCount": wave_count,
        }
    });
    if let Some(sc) = selected_count {
        status["status"]["selectedCount"] = serde_json::json!(sc);
    }
    if let Some(cr) = cluster_ref {
        status["status"]["clusterRef"] = serde_json::json!(cr);
    }
    api.patch_status(name, &Default::default(), &Patch::Merge(&status))
        .await?;
    Ok(())
}

/// Apply a shell MigrationPlan so the controller can populate waves (legacy assessment ref only).
pub async fn ensure_plan_for_assessment(
    client: &Client,
    ns: &str,
    assessment_name: &str,
) -> anyhow::Result<()> {
    if !auto_migration_plan_enabled() {
        return Ok(());
    }
    let plan_name = plan_name_for_assessment(assessment_name);
    let plan = MigrationPlan::new(
        &plan_name,
        MigrationPlanSpec {
            assessment_ref: Some(assessment_name.to_string()),
            selected_namespaces: vec![],
            cluster_ref: Some(cluster_ref_from_env()),
            display_name: None,
            target_mesh_mode: "ambient".into(),
            mesh_target: None,
            ambient_ingress_gateway: None,
            waves: vec![],
        },
    );
    let api: Api<MigrationPlan> = Api::namespaced(client.clone(), ns);
    let pp = PatchParams::apply(FIELD_MANAGER).force();
    api.patch(&plan_name, &pp, &Patch::Apply(&plan)).await?;
    Ok(())
}

fn auto_migration_plan_enabled() -> bool {
    std::env::var("AMBIENTOR_AUTO_MIGRATION_PLAN")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}
