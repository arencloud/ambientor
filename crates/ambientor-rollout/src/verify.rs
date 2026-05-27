use ambientor_mesh::dynamic::{api_resource, list_cr_in_namespace};
use ambientor_mesh::mesh_instances::namespace_enrolled_on_mesh;
use ambientor_types::MeshInstance;
use k8s_openapi::api::core::v1::Namespace;
use kube::api::DynamicObject;
use kube::{Api, Client};
use serde_json::Value;

use crate::engine::RolloutError;
use crate::waypoint::WAYPOINT_GATEWAY_NAME;

/// Verify ambient enrollment, waypoint binding, and waypoint Gateway readiness.
pub async fn verify_namespace_traffic(
    client: &Client,
    namespace: &str,
    mesh: &MeshInstance,
) -> Result<(), RolloutError> {
    verify_namespace_labels(client, namespace, mesh).await?;
    verify_waypoint_gateway(client, namespace).await?;
    verify_no_pending_virtual_services(client, namespace).await?;
    Ok(())
}

async fn verify_namespace_labels(
    client: &Client,
    namespace: &str,
    mesh: &MeshInstance,
) -> Result<(), RolloutError> {
    let api: Api<Namespace> = Api::all(client.clone());
    let ns = api.get(namespace).await.map_err(RolloutError::Kube)?;
    let labels = ns.metadata.labels.unwrap_or_default();
    if labels.get("istio.io/dataplane-mode").map(String::as_str) != Some("ambient") {
        return Err(RolloutError::ExecutionFailed(format!(
            "namespace {namespace} missing istio.io/dataplane-mode=ambient"
        )));
    }
    if !namespace_enrolled_on_mesh(&labels, mesh) {
        return Err(RolloutError::ExecutionFailed(format!(
            "namespace {namespace} not enrolled on rollout mesh '{}' (istio-discovery or istio.io/rev)",
            mesh.discovery_label
        )));
    }
    if labels.get("istio.io/use-waypoint").map(String::as_str) != Some(WAYPOINT_GATEWAY_NAME) {
        return Err(RolloutError::ExecutionFailed(format!(
            "namespace {namespace} missing istio.io/use-waypoint={WAYPOINT_GATEWAY_NAME}"
        )));
    }
    Ok(())
}

async fn verify_waypoint_gateway(client: &Client, namespace: &str) -> Result<(), RolloutError> {
    let ar = api_resource("gateway.networking.k8s.io", "v1", "Gateway", "gateways");
    let api = Api::<DynamicObject>::namespaced_with(client.clone(), namespace, &ar);
    let gw = api.get(WAYPOINT_GATEWAY_NAME).await.map_err(|e| match e {
        kube::Error::Api(ref status) if status.code == 404 => RolloutError::ExecutionFailed(
            format!("waypoint Gateway {WAYPOINT_GATEWAY_NAME} not found in {namespace}"),
        ),
        other => RolloutError::Kube(other),
    })?;
    if !gateway_ready(&gw.data) {
        return Err(RolloutError::ExecutionFailed(format!(
            "waypoint Gateway {WAYPOINT_GATEWAY_NAME} in {namespace} is not programmed yet"
        )));
    }
    Ok(())
}

/// True when Gateway status reports programmed or has assigned addresses.
pub fn gateway_ready(data: &Value) -> bool {
    if let Some(addrs) = data
        .get("status")
        .and_then(|s| s.get("addresses"))
        .and_then(|a| a.as_array())
        && !addrs.is_empty()
    {
        return true;
    }
    data.get("status")
        .and_then(|s| s.get("conditions"))
        .and_then(|c| c.as_array())
        .is_some_and(|conds| {
            conds.iter().any(|c| {
                c.get("type").and_then(|t| t.as_str()) == Some("Programmed")
                    && c.get("status").and_then(|s| s.as_str()) == Some("True")
            })
        })
}

async fn verify_no_pending_virtual_services(
    client: &Client,
    namespace: &str,
) -> Result<(), RolloutError> {
    let vs_ar = api_resource(
        "networking.istio.io",
        "v1",
        "VirtualService",
        "virtualservices",
    );
    let vs_list = list_cr_in_namespace(client, &vs_ar, namespace)
        .await
        .map_err(|e| {
            RolloutError::ExecutionFailed(format!("list VirtualServices in {namespace}: {e}"))
        })?;
    if vs_list.is_empty() {
        return Ok(());
    }
    let hr_ar = api_resource("gateway.networking.k8s.io", "v1", "HTTPRoute", "httproutes");
    let routes = list_cr_in_namespace(client, &hr_ar, namespace)
        .await
        .map_err(|e| {
            RolloutError::ExecutionFailed(format!("list HTTPRoutes in {namespace}: {e}"))
        })?;
    let translated: std::collections::HashSet<_> = routes
        .iter()
        .filter_map(|r| {
            r.metadata
                .labels
                .as_ref()
                .and_then(|l| l.get("ambientor.io/source-name"))
                .cloned()
        })
        .collect();
    let missing: Vec<String> = vs_list
        .iter()
        .filter_map(|vs| vs.metadata.name.clone())
        .filter(|name| !translated.contains(name))
        .collect();
    if !missing.is_empty() {
        return Err(RolloutError::ExecutionFailed(format!(
            "VirtualServices without applied HTTPRoute translation: {}",
            missing.join(", ")
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gateway_ready_when_programmed() {
        let data = json!({
            "status": {
                "conditions": [{
                    "type": "Programmed",
                    "status": "True"
                }]
            }
        });
        assert!(gateway_ready(&data));
    }

    #[test]
    fn gateway_ready_when_addresses_populated() {
        let data = json!({
            "status": {
                "addresses": [{ "value": "10.0.0.1" }]
            }
        });
        assert!(gateway_ready(&data));
    }

    #[test]
    fn gateway_not_ready_without_status() {
        assert!(!gateway_ready(&json!({})));
    }
}
