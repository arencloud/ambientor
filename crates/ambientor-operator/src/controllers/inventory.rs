use std::sync::Arc;

use ambientor_types::{AmbientAssessment, AmbientAssessmentSpec, MeshInventory};
use chrono::Utc;
use futures::StreamExt;
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
    runtime::controller::{Action, Controller},
    runtime::watcher::Config,
};
use tracing::info;

use super::runtime::{ReconcileError, ReconcileResult, error_policy};

pub(crate) const FIELD_MANAGER: &str = "ambientor-operator";

/// Returns true when `triggerScan` is set and the object spec generation changed since last scan.
pub(crate) fn needs_scan(inv: &MeshInventory) -> bool {
    if !inv.spec.trigger_scan {
        return false;
    }
    let meta_gen = inv.metadata.generation.unwrap_or(0);
    let observed = inv
        .status
        .as_ref()
        .map(|s| s.observed_generation)
        .unwrap_or(0);
    meta_gen != observed
}

pub fn assessment_name_for(inv: &MeshInventory) -> String {
    format!(
        "{}-assessment",
        inv.metadata.name.as_deref().unwrap_or("scan")
    )
}

pub async fn run(client: Client) {
    Controller::new(Api::<MeshInventory>::all(client.clone()), Config::default())
        .shutdown_on_signal()
        .run(reconcile, error_policy, Arc::new(client))
        .for_each(|res| async move {
            if let Err(e) = res {
                tracing::error!(error = ?e, "meshinventory controller error");
            }
        })
        .await;
}

async fn reconcile(inv: Arc<MeshInventory>, client: Arc<Client>) -> ReconcileResult {
    reconcile_inner(&client, &inv)
        .await
        .map_err(ReconcileError::Other)?;
    Ok(Action::await_change())
}

async fn reconcile_inner(client: &Client, inv: &MeshInventory) -> anyhow::Result<()> {
    let ns = inv
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());
    let name = inv
        .metadata
        .name
        .clone()
        .ok_or_else(|| anyhow::anyhow!("MeshInventory missing metadata.name"))?;

    if !inv.spec.trigger_scan {
        patch_inventory_status(client, &ns, &name, inv, "Idle", None).await?;
        return Ok(());
    }

    let meta_gen = inv.metadata.generation.unwrap_or(0);

    if !needs_scan(inv) {
        return Ok(());
    }

    let assessment_name = assessment_name_for(inv);
    info!(
        inventory = %name,
        assessment = %assessment_name,
        generation = meta_gen,
        "triggering assessment from inventory"
    );

    let assessment = AmbientAssessment::new(
        &assessment_name,
        AmbientAssessmentSpec {
            inventory_ref: Some(name.clone()),
            cluster_ref: inv.spec.cluster_ref.clone(),
        },
    );

    let assessment_api: Api<AmbientAssessment> = Api::namespaced(client.clone(), &ns);
    let pp = PatchParams::apply(FIELD_MANAGER).force();
    assessment_api
        .patch(&assessment_name, &pp, &Patch::Apply(&assessment))
        .await?;

    let pending_status = serde_json::json!({
        "status": {
            "phase": "Pending",
            "readinessScore": 0,
            "sidecarDependencyScore": 0,
            "trafficCompatibilityScore": 0,
            "overallScore": 0,
            "findings": [],
            "summary": { "blockers": 0, "warnings": 0, "info": 0 }
        }
    });
    assessment_api
        .patch_status(
            &assessment_name,
            &Default::default(),
            &Patch::Merge(&pending_status),
        )
        .await?;

    patch_inventory_status(
        client,
        &ns,
        &name,
        inv,
        "ScanTriggered",
        Some((meta_gen, assessment_name)),
    )
    .await?;

    Ok(())
}

async fn patch_inventory_status(
    client: &Client,
    ns: &str,
    name: &str,
    inv: &MeshInventory,
    phase: &str,
    scan: Option<(i64, String)>,
) -> anyhow::Result<()> {
    let inv_api: Api<MeshInventory> = Api::namespaced(client.clone(), ns);
    let (observed_generation, assessment_ref, last_scan_time) = match scan {
        Some((observed_gen, aref)) => (observed_gen, Some(aref), Some(Utc::now().to_rfc3339())),
        None => (
            inv.status
                .as_ref()
                .map(|s| s.observed_generation)
                .unwrap_or(0),
            inv.status.as_ref().and_then(|s| s.assessment_ref.clone()),
            inv.status
                .as_ref()
                .and_then(|s| s.last_scan_time.map(|t| t.to_rfc3339())),
        ),
    };
    let status = serde_json::json!({
        "status": {
            "phase": phase,
            "generation": inv.status.as_ref().map(|s| s.generation + 1).unwrap_or(1),
            "observedGeneration": observed_generation,
            "lastScanTime": last_scan_time,
            "assessmentRef": assessment_ref,
        }
    });
    inv_api
        .patch_status(name, &Default::default(), &Patch::Merge(&status))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use ambientor_types::MeshInventory;

    use super::*;

    fn inv_with(trigger: bool, meta_gen: i64, observed: i64) -> MeshInventory {
        MeshInventory {
            spec: ambientor_types::MeshInventorySpec {
                trigger_scan: trigger,
                cluster_ref: None,
                namespace_selector: None,
            },
            status: Some(ambientor_types::MeshInventoryStatus {
                phase: String::new(),
                generation: 0,
                observed_generation: observed,
                last_scan_time: None,
                assessment_ref: None,
            }),
            metadata: kube::api::ObjectMeta {
                generation: Some(meta_gen),
                ..Default::default()
            },
        }
    }

    #[test]
    fn needs_scan_when_generation_advances() {
        assert!(needs_scan(&inv_with(true, 2, 1)));
        assert!(!needs_scan(&inv_with(true, 2, 2)));
        assert!(!needs_scan(&inv_with(false, 2, 1)));
    }
}
