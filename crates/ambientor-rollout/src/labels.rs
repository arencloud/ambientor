use k8s_openapi::api::core::v1::Namespace;
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
};
use serde_json::json;
use tracing::info;

use crate::engine::{FIELD_MANAGER, RolloutError};

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
    let pp = PatchParams::apply(FIELD_MANAGER).force();
    api.patch(name, &pp, &Patch::Apply(patch)).await?;
    Ok(())
}
