use ambientor_core::scoring::compute_scores;
use ambientor_k8s::detect_platform;
use ambientor_mesh::backend::backend_for_flavor;
use ambientor_scan::default_registry;
use ambientor_types::{AmbientAssessment, FindingSummary};
use kube::{Api, Client, api::Patch};

use super::requeue_interval;

pub async fn run(client: Client) {
    loop {
        if let Err(e) = reconcile_all(&client).await {
            tracing::error!(error = %e, "assessment reconcile failed");
        }
        tokio::time::sleep(requeue_interval()).await;
    }
}

async fn reconcile_all(client: &Client) -> anyhow::Result<()> {
    let api: Api<AmbientAssessment> = Api::all(client.clone());
    let list = api.list(&Default::default()).await?;
    for obj in list.items {
        let phase = obj.status.as_ref().map(|s| s.phase.as_str()).unwrap_or("");
        if phase != "Completed" {
            reconcile_one(client, &obj).await?;
        }
    }
    Ok(())
}

async fn reconcile_one(client: &Client, obj: &AmbientAssessment) -> anyhow::Result<()> {
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
