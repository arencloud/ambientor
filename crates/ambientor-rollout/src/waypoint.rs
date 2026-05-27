use std::time::Duration;

use ambientor_mesh::dynamic::api_resource;
use k8s_openapi::api::core::v1::Namespace;
use kube::api::DynamicObject;
use kube::{
    Api, Client,
    api::{DeleteParams, Patch, PatchParams},
};
use serde_json::json;
use tokio::time::sleep;
use tracing::info;

use crate::apply::apply_namespaced_manifest;
use crate::engine::RolloutError;
use crate::labels::unlabel_namespace_use_waypoint;
use ambientor_types::MeshInstance;

use crate::preflight::{preflight_before_deploy_waypoint, waypoint_gateway_stuck_message};
use crate::verify::gateway_ready;

const GATEWAY_READY_TIMEOUT_SECS: u64 = 120;
const GATEWAY_POLL_INTERVAL_SECS: u64 = 2;

pub const WAYPOINT_GATEWAY_NAME: &str = "waypoint";
const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";
const MANAGED_BY_VALUE: &str = "ambientor";

/// Deploy an Istio ambient waypoint (`Gateway` + `istio.io/use-waypoint` on the namespace).
pub async fn deploy_waypoint(
    client: &Client,
    namespace: &str,
    mesh: &MeshInstance,
) -> Result<(), RolloutError> {
    preflight_before_deploy_waypoint(client, namespace, mesh).await?;
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
    wait_gateway_programmed(client, namespace, mesh).await?;
    info!(namespace = %namespace, waypoint = %WAYPOINT_GATEWAY_NAME, "deployed ambient waypoint");
    Ok(())
}

async fn wait_gateway_programmed(
    client: &Client,
    namespace: &str,
    mesh: &MeshInstance,
) -> Result<(), RolloutError> {
    let ar = api_resource("gateway.networking.k8s.io", "v1", "Gateway", "gateways");
    let api = Api::<DynamicObject>::namespaced_with(client.clone(), namespace, &ar);
    let deadline = Duration::from_secs(GATEWAY_READY_TIMEOUT_SECS);
    let started = std::time::Instant::now();
    let mut last_data: Option<serde_json::Value> = None;
    while started.elapsed() < deadline {
        match api.get(WAYPOINT_GATEWAY_NAME).await {
            Ok(gw) if gateway_ready(&gw.data) => return Ok(()),
            Ok(gw) => {
                last_data = Some(gw.data);
            }
            Err(kube::Error::Api(e)) if e.code == 404 => {}
            Err(e) => return Err(RolloutError::Kube(e)),
        }
        sleep(Duration::from_secs(GATEWAY_POLL_INTERVAL_SECS)).await;
    }
    Err(RolloutError::ExecutionFailed(waypoint_gateway_stuck_message(
        namespace,
        mesh,
        last_data.as_ref(),
    )))
}

/// Remove waypoint Gateway (if managed by Ambientor) and `istio.io/use-waypoint` label.
pub async fn revert_waypoint(client: &Client, namespace: &str) -> Result<(), RolloutError> {
    delete_managed_waypoint_gateway(client, namespace).await?;
    unlabel_namespace_use_waypoint(client, namespace).await?;
    info!(namespace = %namespace, "reverted ambient waypoint");
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
    api.patch(namespace, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
}

async fn delete_managed_waypoint_gateway(
    client: &Client,
    namespace: &str,
) -> Result<(), RolloutError> {
    let ar = api_resource("gateway.networking.k8s.io", "v1", "Gateway", "gateways");
    let api = Api::<DynamicObject>::namespaced_with(client.clone(), namespace, &ar);
    let gw = match api.get(WAYPOINT_GATEWAY_NAME).await {
        Ok(g) => g,
        Err(kube::Error::Api(e)) if e.code == 404 => return Ok(()),
        Err(e) => return Err(RolloutError::Kube(e)),
    };
    let managed = gw
        .metadata
        .labels
        .as_ref()
        .and_then(|l| l.get(MANAGED_BY_LABEL))
        .map(String::as_str)
        == Some(MANAGED_BY_VALUE);
    if !managed {
        info!(
            namespace = %namespace,
            gateway = %WAYPOINT_GATEWAY_NAME,
            "skipping waypoint delete: not managed by ambientor"
        );
        return Ok(());
    }
    api.delete(WAYPOINT_GATEWAY_NAME, &DeleteParams::default())
        .await?;
    Ok(())
}
