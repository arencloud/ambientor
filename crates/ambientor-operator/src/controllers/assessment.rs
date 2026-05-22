use std::sync::Arc;

use ambientor_core::scoring::compute_scores;
use ambientor_k8s::detect_platform;
use ambientor_mesh::backend::backend_for_flavor;
use ambientor_scan::default_registry;
use ambientor_types::{AmbientAssessment, FindingSummary};
use futures::StreamExt;
use kube::{
    Api, Client,
    api::Patch,
    runtime::controller::{Action, Controller},
    runtime::watcher::Config,
};

use super::runtime::{ReconcileError, ReconcileResult, error_policy};

pub async fn run(client: Client) {
    Controller::new(
        Api::<AmbientAssessment>::all(client.clone()),
        Config::default(),
    )
    .shutdown_on_signal()
    .run(reconcile, error_policy, Arc::new(client))
    .for_each(|res| async move {
        if let Err(e) = res {
            tracing::error!(error = %e, "ambientassessment controller error");
        }
    })
    .await;
}

async fn reconcile(obj: Arc<AmbientAssessment>, client: Arc<Client>) -> ReconcileResult {
    let phase = obj.status.as_ref().map(|s| s.phase.as_str()).unwrap_or("");
    if phase == "Completed" {
        return Ok(Action::await_change());
    }
    reconcile_inner(&client, &obj)
        .await
        .map_err(ReconcileError::Other)?;
    Ok(Action::await_change())
}

async fn reconcile_inner(client: &Client, obj: &AmbientAssessment) -> anyhow::Result<()> {
    let platform = detect_platform(client).await.unwrap_or_default();
    let backend = backend_for_flavor(platform.mesh_flavor);
    let ctx = backend.build_rule_context(client).await.unwrap_or_default();
    let findings = default_registry().evaluate_all(&ctx);
    let scores = compute_scores(&findings);
    let summary = FindingSummary::from_findings(&findings);

    let ns = obj
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());
    let api: Api<AmbientAssessment> = Api::namespaced(client.clone(), &ns);
    if let Some(name) = &obj.metadata.name {
        let status = serde_json::json!({
            "status": {
                "phase": "Completed",
                "readinessScore": scores.readiness,
                "sidecarDependencyScore": scores.sidecar_dependency,
                "trafficCompatibilityScore": scores.traffic_compatibility,
                "overallScore": scores.overall,
                "findings": findings,
                "summary": summary,
            }
        });
        api.patch_status(name, &Default::default(), &Patch::Merge(status))
            .await?;
    }
    Ok(())
}
