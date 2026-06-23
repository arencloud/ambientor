use std::sync::Arc;
use std::time::Duration;

use ambientor_k8s::client_for_cluster_ref;
use ambientor_mesh::mesh_instances::{discover_mesh_instances, resolve_mesh_target};
use ambientor_rollout::audit::audit_from_rollout_event;
use ambientor_types::Rollout;
use futures::StreamExt;
use kube::{
    Api,
    api::Patch,
    runtime::controller::{Action, Controller},
    runtime::watcher::Config,
};

use super::context::OperatorContext;
use super::dashboard;
use super::runtime::{ReconcileError, ReconcileResult, error_policy};

pub async fn run(ctx: Arc<OperatorContext>) {
    let client = ctx.client.clone();
    Controller::new(Api::<Rollout>::all(client.clone()), Config::default())
        .shutdown_on_signal()
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(e) = res {
                tracing::error!(error = ?e, "rollout controller error");
            }
        })
        .await;
}

async fn reconcile(obj: Arc<Rollout>, ctx: Arc<OperatorContext>) -> ReconcileResult {
    let phase = obj.status.as_ref().map(|s| s.phase.as_str()).unwrap_or("");
    if phase == "Completed" || phase == "Failed" || phase == "RolledBack" {
        sync_dashboard(&ctx, obj.spec.cluster_ref.as_deref()).await;
        return Ok(Action::await_change());
    }
    let mut status = obj.status.clone().unwrap_or_default();
    reconcile_inner(&ctx, &obj, &mut status)
        .await
        .map_err(ReconcileError::Other)?;
    sync_dashboard(&ctx, obj.spec.cluster_ref.as_deref()).await;
    if status.phase == "AwaitingApproval" {
        Ok(Action::await_change())
    } else {
        Ok(Action::requeue(Duration::from_secs(10)))
    }
}

async fn sync_dashboard(ctx: &OperatorContext, cluster_ref: Option<&str>) {
    let Some(store) = ctx.dashboard_repo.as_ref() else {
        return;
    };
    dashboard::sync_cluster_ref_now(
        &ctx.client,
        store.as_ref(),
        ctx.scan_repo.as_deref(),
        cluster_ref,
    )
    .await;
}

async fn reconcile_inner(
    ctx: &OperatorContext,
    obj: &Rollout,
    status: &mut ambientor_types::RolloutStatus,
) -> anyhow::Result<()> {
    let cluster_ref = obj.spec.cluster_ref.as_deref();
    let target_client = client_for_cluster_ref(&ctx.client, cluster_ref)
        .await
        .map_err(|e| anyhow::anyhow!("rollout target cluster client: {e}"))?;

    let instances = discover_mesh_instances(&target_client)
        .await
        .map_err(|e| anyhow::anyhow!("discover mesh instances: {e}"))?;
    let mesh = resolve_mesh_target(&instances, obj.spec.mesh_target.as_ref())
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let events = ctx
        .rollout_engine
        .reconcile(&target_client, &obj.spec, status, &mesh)
        .await
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    let ns = obj
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());
    let name = obj
        .metadata
        .name
        .clone()
        .unwrap_or_else(|| "unknown".into());

    if let Some(repo) = &ctx.audit_repo {
        for event in &events {
            let audit = audit_from_rollout_event(&ns, &name, "operator", event);
            if let Err(e) = repo.append(&audit).await {
                tracing::warn!(
                    error = %e,
                    rollout = %name,
                    action = %audit.action,
                    "failed to append rollout audit event"
                );
            }
        }
    }

    let api: Api<Rollout> = Api::namespaced(ctx.client.clone(), &ns);
    if let Some(name) = &obj.metadata.name {
        // Approval may be patched via API/CLI while reconcile runs; do not clobber it.
        if let Ok(latest) = api.get(name).await
            && let Some(latest_status) = latest.status.as_ref()
        {
            status.approved_stage = status.approved_stage.max(latest_status.approved_stage);
        }
        let patch = serde_json::json!({ "status": status });
        api.patch_status(name, &Default::default(), &Patch::Merge(patch))
            .await?;
    }
    Ok(())
}
