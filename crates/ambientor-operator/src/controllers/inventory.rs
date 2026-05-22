use ambientor_types::{AmbientAssessment, MeshInventory};
use chrono::Utc;
use kube::{
    Api, Client,
    api::{ListParams, Patch, PatchParams},
};
use tracing::info;

use super::requeue_interval;

pub async fn run(client: Client) {
    loop {
        if let Err(e) = reconcile_all(&client).await {
            tracing::error!(error = %e, "meshinventory reconcile failed");
        }
        tokio::time::sleep(requeue_interval()).await;
    }
}

async fn reconcile_all(client: &Client) -> anyhow::Result<()> {
    let api: Api<MeshInventory> = Api::all(client.clone());
    let list = api.list(&ListParams::default()).await?;
    for inv in list.items {
        if !inv.spec.trigger_scan {
            continue;
        }
        reconcile_one(client, &inv).await?;
    }
    Ok(())
}

async fn reconcile_one(client: &Client, inv: &MeshInventory) -> anyhow::Result<()> {
    info!(name = ?inv.metadata.name, "triggering assessment from inventory");
    let assessment_name = format!(
        "{}-{}",
        inv.metadata.name.as_deref().unwrap_or("scan"),
        Utc::now().timestamp()
    );
    let ns = inv
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());

    let assessment = AmbientAssessment::new(
        &assessment_name,
        ambientor_types::AmbientAssessmentSpec {
            inventory_ref: inv.metadata.name.clone(),
            cluster_ref: inv.spec.cluster_ref.clone(),
        },
    );
    let assessment_api: Api<AmbientAssessment> = Api::namespaced(client.clone(), &ns);
    let pp = PatchParams::apply("ambientor-operator").force();
    assessment_api
        .patch(&assessment_name, &pp, &Patch::Apply(&assessment))
        .await?;

    let inv_api: Api<MeshInventory> = Api::namespaced(client.clone(), &ns);
    if let Some(name) = &inv.metadata.name {
        let status = serde_json::json!({
            "status": {
                "phase": "ScanTriggered",
                "generation": inv.status.as_ref().map(|s| s.generation + 1).unwrap_or(1),
                "lastScanTime": Utc::now().to_rfc3339(),
                "assessmentRef": assessment_name,
            }
        });
        inv_api
            .patch_status(name, &Default::default(), &Patch::Merge(status))
            .await?;
    }
    Ok(())
}
