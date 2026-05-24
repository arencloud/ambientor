use std::sync::Arc;

use ambientor_plan::{
    assessment_result_from_status, build_plan, namespaces_for_planning,
    namespaces_matching_selector, plan_name_for_assessment,
};
use ambientor_types::{AmbientAssessment, MeshInventory, MigrationPlan, MigrationPlanSpec};
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

pub async fn run(client: Client) {
    Controller::new(Api::<MigrationPlan>::all(client.clone()), Config::default())
        .shutdown_on_signal()
        .run(reconcile, error_policy, Arc::new(client))
        .for_each(|res| async move {
            if let Err(e) = res {
                tracing::error!(error = %e, "migrationplan controller error");
            }
        })
        .await;
}

async fn reconcile(plan: Arc<MigrationPlan>, client: Arc<Client>) -> ReconcileResult {
    if plan.status.as_ref().is_some_and(|s| s.approved) {
        return Ok(Action::await_change());
    }
    reconcile_inner(&client, &plan)
        .await
        .map_err(ReconcileError::Other)?;
    Ok(Action::await_change())
}

async fn reconcile_inner(client: &Client, plan: &MigrationPlan) -> anyhow::Result<()> {
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

    let assessment_ref =
        plan.spec.assessment_ref.clone().ok_or_else(|| {
            anyhow::anyhow!("MigrationPlan {plan_name} missing spec.assessmentRef")
        })?;

    let assess_api: Api<AmbientAssessment> = Api::namespaced(client.clone(), &ns);
    let assessment = match assess_api.get(&assessment_ref).await {
        Ok(a) => a,
        Err(kube::Error::Api(e)) if e.code == 404 => {
            patch_plan_status(client, &ns, &plan_name, "Failed", 0).await?;
            return Ok(());
        }
        Err(e) => return Err(e.into()),
    };

    let status = assessment
        .status
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("assessment {assessment_ref} has no status"))?;

    if status.phase != "Completed" {
        patch_plan_status(client, &ns, &plan_name, "WaitingForAssessment", 0).await?;
        return Ok(());
    }

    let assessment_result = assessment_result_from_status(status);
    let inventory_namespaces = inventory_target_namespaces(client, &assessment, &ns)
        .await
        .unwrap_or_default();
    let namespaces = namespaces_for_planning(&assessment_result.findings, &inventory_namespaces);
    let built = build_plan(&assessment_result, &namespaces);
    let spec = MigrationPlanSpec {
        assessment_ref: Some(assessment_ref.clone()),
        target_mesh_mode: built.target_mesh_mode,
        waves: built.waves,
    };

    let plan_api: Api<MigrationPlan> = Api::namespaced(client.clone(), &ns);
    let updated = MigrationPlan::new(&plan_name, spec);
    let pp = PatchParams::apply(FIELD_MANAGER).force();
    plan_api
        .patch(&plan_name, &pp, &Patch::Apply(&updated))
        .await?;

    let wave_count = plan_api.get(&plan_name).await?.spec.waves.len() as i32;
    patch_plan_status(client, &ns, &plan_name, "Ready", wave_count).await?;

    let plan = plan_api.get(&plan_name).await?;
    for wave in &plan.spec.waves {
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

    info!(
        plan = %plan_name,
        assessment = %assessment_ref,
        waves = wave_count,
        "migration plan reconciled"
    );
    Ok(())
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
) -> anyhow::Result<()> {
    let api: Api<MigrationPlan> = Api::namespaced(client.clone(), ns);
    let status = serde_json::json!({
        "status": {
            "phase": phase,
            "approved": false,
            "waveCount": wave_count,
        }
    });
    api.patch_status(name, &Default::default(), &Patch::Merge(&status))
        .await?;
    Ok(())
}

/// Apply a shell MigrationPlan so the controller can populate waves.
pub async fn ensure_plan_for_assessment(
    client: &Client,
    ns: &str,
    assessment_name: &str,
) -> anyhow::Result<()> {
    let plan_name = plan_name_for_assessment(assessment_name);
    let plan = MigrationPlan::new(
        &plan_name,
        MigrationPlanSpec {
            assessment_ref: Some(assessment_name.to_string()),
            target_mesh_mode: "ambient".into(),
            waves: vec![],
        },
    );
    let api: Api<MigrationPlan> = Api::namespaced(client.clone(), ns);
    let pp = PatchParams::apply(FIELD_MANAGER).force();
    api.patch(&plan_name, &pp, &Patch::Apply(&plan)).await?;
    Ok(())
}
