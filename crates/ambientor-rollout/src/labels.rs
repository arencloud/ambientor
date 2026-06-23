use k8s_openapi::api::core::v1::Namespace;
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
};
use serde_json::json;
use std::collections::BTreeMap;
use tracing::info;

use crate::engine::RolloutError;
use ambientor_mesh::enrollment_labels_to_apply;
use ambientor_types::MeshInstance;

pub const PRE_MIGRATION_LABELS_ANNOTATION: &str = "ambientor.io/pre-migration-labels";

/// Capture namespace labels before the first mutating rollout stage so rollback can restore exactly.
pub async fn snapshot_namespace_pre_migration(
    client: &Client,
    name: &str,
) -> Result<(), RolloutError> {
    let api: Api<Namespace> = Api::all(client.clone());
    let ns = api.get(name).await.map_err(RolloutError::Kube)?;
    if ns
        .metadata
        .annotations
        .as_ref()
        .is_some_and(|a| a.contains_key(PRE_MIGRATION_LABELS_ANNOTATION))
    {
        return Ok(());
    }
    let labels = ns.metadata.labels.unwrap_or_default();
    let encoded =
        serde_json::to_string(&labels).map_err(|e| RolloutError::ExecutionFailed(e.to_string()))?;
    let patch = json!({
        "metadata": {
            "annotations": {
                PRE_MIGRATION_LABELS_ANNOTATION: encoded
            }
        }
    });
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    info!(namespace = %name, "snapshotted pre-migration namespace labels");
    Ok(())
}

/// Restore labels from the pre-migration snapshot and clear the annotation.
/// Merge patch cannot remove keys absent from the snapshot; explicitly null migration labels.
pub async fn restore_namespace_pre_migration(
    client: &Client,
    name: &str,
) -> Result<bool, RolloutError> {
    let api: Api<Namespace> = Api::all(client.clone());
    let ns = api.get(name).await.map_err(RolloutError::Kube)?;
    let Some(encoded) = ns
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(PRE_MIGRATION_LABELS_ANNOTATION))
    else {
        return Ok(false);
    };
    let snapshot: BTreeMap<String, String> =
        serde_json::from_str(encoded).map_err(|e| RolloutError::ExecutionFailed(e.to_string()))?;
    let current = ns.metadata.labels.unwrap_or_default();

    let mut labels = serde_json::Map::new();
    for (k, v) in &snapshot {
        labels.insert(k.clone(), json!(v));
    }
    for k in current.keys() {
        if !snapshot.contains_key(k) && migration_managed_label(k) {
            labels.insert(k.clone(), serde_json::Value::Null);
        }
    }

    let patch = json!({
        "metadata": {
            "labels": labels,
            "annotations": {
                PRE_MIGRATION_LABELS_ANNOTATION: null,
                "istio.io/use-waypoint": null
            }
        }
    });
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    info!(namespace = %name, "restored pre-migration namespace labels");
    Ok(true)
}

fn migration_managed_label(key: &str) -> bool {
    matches!(
        key,
        "istio.io/dataplane-mode"
            | "istio-injection"
            | "istio.io/use-waypoint"
            | "istio.io/rev"
            | "istio-discovery"
    )
}

pub async fn label_namespace_ambient(client: &Client, name: &str) -> Result<(), RolloutError> {
    patch_namespace_labels(
        client,
        name,
        json!({
            "istio.io/dataplane-mode": "ambient",
            "istio-injection": null,
        }),
    )
    .await?;
    info!(namespace = %name, "labeled namespace for ambient");
    Ok(())
}

pub async fn unlabel_namespace_ambient(client: &Client, name: &str) -> Result<(), RolloutError> {
    patch_namespace_labels(
        client,
        name,
        json!({
            "istio.io/dataplane-mode": null
        }),
    )
    .await?;
    info!(namespace = %name, "removed ambient dataplane label");
    Ok(())
}

pub async fn restore_namespace_injection(client: &Client, name: &str) -> Result<(), RolloutError> {
    // Revision-based meshes (OpenShift OSSM) use istio.io/rev, not istio-injection.
    // Pre-migration snapshot restore in finalize_rollback_namespaces handles sidecar labels.
    let _ = (client, name);
    Ok(())
}

pub async fn remove_namespace_injection(client: &Client, name: &str) -> Result<(), RolloutError> {
    let api: Api<Namespace> = Api::all(client.clone());
    let patch = json!({
        "metadata": {
            "labels": {
                "istio-injection": null
            },
            "annotations": { "sidecar.istio.io/inject": null }
        }
    });
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    info!(namespace = %name, "removed sidecar injection labels and annotations");
    Ok(())
}

/// Drop revision tag from the namespace so new pods are not sidecar-injected (ambient uses ztunnel).
pub async fn clear_namespace_revision_label(
    client: &Client,
    name: &str,
) -> Result<(), RolloutError> {
    let api: Api<Namespace> = Api::all(client.clone());
    let patch = json!({
        "metadata": {
            "labels": {
                "istio.io/rev": null
            }
        }
    });
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    info!(namespace = %name, "cleared istio.io/rev for ambient workloads");
    Ok(())
}

pub async fn unlabel_namespace_use_waypoint(
    client: &Client,
    name: &str,
) -> Result<(), RolloutError> {
    patch_namespace_labels(
        client,
        name,
        json!({
            "istio.io/use-waypoint": null
        }),
    )
    .await?;
    info!(namespace = %name, "removed use-waypoint label");
    Ok(())
}

/// Re-apply istiod discovery/revision labels before Gateway programming.
pub async fn ensure_mesh_enrollment_labels(
    client: &Client,
    name: &str,
    mesh: &MeshInstance,
) -> Result<(), RolloutError> {
    let labels = enrollment_labels_to_apply(mesh);
    if labels.is_empty() {
        return Ok(());
    }
    patch_namespace_labels(client, name, serde_json::json!(labels)).await
}

async fn patch_namespace_labels(
    client: &Client,
    name: &str,
    labels: serde_json::Value,
) -> Result<(), RolloutError> {
    let api: Api<Namespace> = Api::all(client.clone());
    let patch = json!({
        "metadata": { "labels": labels }
    });
    // Merge patch: SSA Apply requires apiVersion/kind on the patch body.
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
}
