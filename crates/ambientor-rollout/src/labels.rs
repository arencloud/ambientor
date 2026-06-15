use k8s_openapi::api::core::v1::Namespace;
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
};
use serde_json::json;
use tracing::info;

use crate::engine::RolloutError;

pub async fn label_namespace_ambient(client: &Client, name: &str) -> Result<(), RolloutError> {
    patch_namespace_labels(
        client,
        name,
        json!({
            "istio.io/dataplane-mode": "ambient",
            "istio-injection": null
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
    patch_namespace_labels(
        client,
        name,
        json!({
            "istio-injection": "enabled"
        }),
    )
    .await?;
    info!(namespace = %name, "restored sidecar injection label");
    Ok(())
}

pub async fn remove_namespace_injection(client: &Client, name: &str) -> Result<(), RolloutError> {
    let api: Api<Namespace> = Api::all(client.clone());
    let patch = json!({
        "metadata": {
            "labels": { "istio-injection": null },
            "annotations": { "sidecar.istio.io/inject": null }
        }
    });
    api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    info!(namespace = %name, "removed sidecar injection labels/annotations");
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
