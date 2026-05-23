use k8s_openapi::api::core::v1::Namespace;
use kube::{
    Api, Client,
    api::{Patch, PatchParams},
};
use serde_json::json;
use tracing::info;

use crate::apply::apply_namespaced_manifest;
use crate::engine::{FIELD_MANAGER, RolloutError};

pub const WAYPOINT_GATEWAY_NAME: &str = "waypoint";

/// Deploy an Istio ambient waypoint (`Gateway` + `istio.io/use-waypoint` on the namespace).
pub async fn deploy_waypoint(client: &Client, namespace: &str) -> Result<(), RolloutError> {
    let manifest = json!({
        "apiVersion": "gateway.networking.k8s.io/v1",
        "kind": "Gateway",
        "metadata": {
            "name": WAYPOINT_GATEWAY_NAME,
            "namespace": namespace,
            "labels": {
                "istio.io/waypoint-for": "service",
                "app.kubernetes.io/managed-by": "ambientor"
            }
        },
        "spec": {
            "gatewayClassName": "istio-waypoint",
            "listeners": [{
                "name": "mesh",
                "port": 15008,
                "protocol": "HBONE"
            }]
        }
    });
    apply_namespaced_manifest(client, namespace, &manifest).await?;
    label_namespace_use_waypoint(client, namespace).await?;
    info!(namespace = %namespace, waypoint = %WAYPOINT_GATEWAY_NAME, "deployed ambient waypoint");
    Ok(())
}

async fn label_namespace_use_waypoint(
    client: &Client,
    namespace: &str,
) -> Result<(), RolloutError> {
    let api: Api<Namespace> = Api::all(client.clone());
    let patch = json!({
        "metadata": {
            "labels": {
                "istio.io/use-waypoint": WAYPOINT_GATEWAY_NAME
            }
        }
    });
    let pp = PatchParams::apply(FIELD_MANAGER).force();
    api.patch(namespace, &pp, &Patch::Apply(patch)).await?;
    Ok(())
}
