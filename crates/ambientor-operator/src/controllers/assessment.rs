use std::sync::Arc;

use ambientor_core::scoring::compute_scores;
use ambientor_db::{
    ApplicationAssessmentStore, DashboardStore, ScanStore, StoredAssessment,
    assessment_sync::persist_full_assessment, cluster_ref_from_env,
};
use ambientor_k8s::{ClusterResourceCache, detect_platform};
use ambientor_mesh::inventory::{self, CoreSnapshot};
use ambientor_scan::default_registry;
use ambientor_types::{AmbientAssessment, FindingSummary};
use futures::StreamExt;
use kube::{
    Api, Client,
    api::Patch,
    runtime::controller::{Action, Controller},
    runtime::watcher::Config,
};

use super::migration_plan::ensure_plan_for_assessment;
use super::runtime::{ReconcileError, ReconcileResult, error_policy};

pub async fn run(
    client: Client,
    scan_repo: Option<Arc<dyn ScanStore>>,
    dashboard_repo: Option<Arc<dyn DashboardStore>>,
    applications_repo: Option<Arc<dyn ApplicationAssessmentStore>>,
    cache: Option<Arc<ClusterResourceCache>>,
) {
    let ctx = Arc::new(AssessmentContext {
        client,
        scan_repo,
        dashboard_repo,
        applications_repo,
        cache,
    });
    Controller::new(
        Api::<AmbientAssessment>::all(ctx.client.clone()),
        Config::default(),
    )
    .shutdown_on_signal()
    .run(reconcile, error_policy, ctx)
    .for_each(|res| async move {
        if let Err(e) = res {
            tracing::error!(error = ?e, "ambientassessment controller error");
        }
    })
    .await;
}

struct AssessmentContext {
    client: Client,
    scan_repo: Option<Arc<dyn ScanStore>>,
    dashboard_repo: Option<Arc<dyn DashboardStore>>,
    applications_repo: Option<Arc<dyn ApplicationAssessmentStore>>,
    cache: Option<Arc<ClusterResourceCache>>,
}

impl AssessmentContext {
    fn core_snapshot(&self) -> Option<CoreSnapshot> {
        let cache = self.cache.as_ref()?;
        if !cache.is_populated() {
            return None;
        }
        Some((cache.pod_snapshot(), cache.namespace_snapshot()))
    }
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
    let rule_ctx =
        inventory::collect_inventory(client, platform.mesh_flavor, assess_ctx.core_snapshot())
            .await
            .unwrap_or_default();
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

        if let Err(e) = ensure_plan_for_assessment(client, &ns, name).await {
            tracing::warn!(error = %e, assessment = %name, "failed to ensure MigrationPlan");
        }

        if let (Some(apps), Some(dash)) = (
            &assess_ctx.applications_repo,
            &assess_ctx.dashboard_repo,
        ) {
            let cluster_ref = cluster_ref_from_env();
            if let Err(e) = persist_full_assessment(
                apps.as_ref(),
                dash.as_ref(),
                client,
                &cluster_ref,
                &rule_ctx,
                &findings,
            )
            .await
            {
                tracing::warn!(error = %e, "failed to persist assessment and dashboard");
            }
        }
    }
    Ok(())
}
