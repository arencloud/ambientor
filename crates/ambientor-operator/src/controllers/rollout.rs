use std::sync::Arc;
use std::time::Duration;

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
use super::runtime::{ReconcileError, ReconcileResult, error_policy};

pub async fn run(ctx: Arc<OperatorContext>) {
    let client = ctx.client.clone();
    Controller::new(Api::<Rollout>::all(client.clone()), Config::default())
        .shutdown_on_signal()
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            if let Err(e) = res {
                tracing::error!(error = %e, "rollout controller error");
            }
        })
        .await;
}

async fn reconcile(obj: Arc<Rollout>, ctx: Arc<OperatorContext>) -> ReconcileResult {
    let phase = obj.status.as_ref().map(|s| s.phase.as_str()).unwrap_or("");
    if phase == "Completed" || phase == "Failed" {
        return Ok(Action::await_change());
    }
    let mut status = obj.status.clone().unwrap_or_default();
    reconcile_inner(&ctx, &obj, &mut status)
        .await
        .map_err(ReconcileError::Other)?;
    if status.phase == "AwaitingApproval" {
        Ok(Action::await_change())
    } else {
        Ok(Action::requeue(Duration::from_secs(10)))
    }
}

async fn reconcile_inner(
    ctx: &OperatorContext,
    obj: &Rollout,
    status: &mut ambientor_types::RolloutStatus,
) -> anyhow::Result<()> {
    let events = ctx
        .rollout_engine
        .reconcile(&obj.spec, status)
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
