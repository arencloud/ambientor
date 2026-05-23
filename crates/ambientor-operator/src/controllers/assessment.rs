use std::sync::Arc;

use ambientor_core::scoring::compute_scores;
use ambientor_db::{ScanRepository, StoredAssessment, cluster_ref_from_env};
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

pub async fn run(client: Client, scan_repo: Option<Arc<ScanRepository>>) {
    let ctx = Arc::new(AssessmentContext { client, scan_repo });
    Controller::new(
        Api::<AmbientAssessment>::all(ctx.client.clone()),
        Config::default(),
    )
    .shutdown_on_signal()
    .run(reconcile, error_policy, ctx)
    .for_each(|res| async move {
        if let Err(e) = res {
            tracing::error!(error = %e, "ambientassessment controller error");
        }
    })
    .await;
}

struct AssessmentContext {
    client: Client,
    scan_repo: Option<Arc<ScanRepository>>,
}

async fn reconcile(obj: Arc<AmbientAssessment>, ctx: Arc<AssessmentContext>) -> ReconcileResult {
    let phase = obj.status.as_ref().map(|s| s.phase.as_str()).unwrap_or("");
    if phase == "Completed" {
        return Ok(Action::await_change());
    }
    reconcile_inner(&ctx, &obj)
        .await
        .map_err(ReconcileError::Other)?;
    Ok(Action::await_change())
}

async fn reconcile_inner(
    assess_ctx: &AssessmentContext,
    obj: &AmbientAssessment,
) -> anyhow::Result<()> {
    let client = &assess_ctx.client;
    let platform = detect_platform(client).await.unwrap_or_default();
    let backend = backend_for_flavor(platform.mesh_flavor);
    let rule_ctx = backend.build_rule_context(client).await.unwrap_or_default();
    let findings = default_registry().evaluate_all(&rule_ctx);
    let scores = compute_scores(&findings);
    let summary = FindingSummary::from_findings(&findings);

    let ns = obj
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());
    let api: Api<AmbientAssessment> = Api::namespaced(client.clone(), &ns);
    if let Some(name) = &obj.metadata.name {
        let payload = StoredAssessment {
            findings: findings.clone(),
            scores: scores.clone(),
            summary: summary.clone(),
            source: Some("operator".into()),
            assessment_name: Some(name.clone()),
        };
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

        if let Some(repo) = &assess_ctx.scan_repo
            && let Err(e) = repo
                .record_completed(&cluster_ref_from_env(), Some(ns.as_str()), &payload)
                .await
        {
            tracing::warn!(error = %e, assessment = %name, "failed to persist scan run");
        }
    }
    Ok(())
}
