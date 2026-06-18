use std::time::Duration;

use ambientor_mesh::dynamic::{api_resource, list_cr_in_namespace};
use ambientor_mesh::namespace_enrolled_on_mesh;
use ambientor_types::MeshInstance;
use k8s_openapi::api::core::v1::Namespace;
use kube::api::DynamicObject;
use kube::{Api, Client};
use serde_json::Value;
use tokio::time::sleep;

use crate::engine::RolloutError;
use crate::waypoint::WAYPOINT_GATEWAY_NAME;

const WORKLOAD_READY_TIMEOUT_SECS: u64 = 180;
const WORKLOAD_POLL_INTERVAL_SECS: u64 = 2;

/// Verify ambient enrollment, waypoint binding, and waypoint Gateway readiness.
pub async fn verify_namespace_traffic(
    client: &Client,
    namespace: &str,
    mesh: &MeshInstance,
) -> Result<(), RolloutError> {
    verify_namespace_labels(client, namespace, mesh).await?;
    verify_waypoint_gateway(client, namespace).await?;
    verify_no_pending_virtual_services(client, namespace).await?;
    verify_external_ingress_routes(client, namespace).await?;
    Ok(())
}

/// Fail when the namespace exposes public hostnames but north–south routes are not attached
/// to a programmed ambient ingress Gateway (common after sidecar→ambient without gateway cutover).
pub async fn verify_external_ingress_routes(
    client: &Client,
    namespace: &str,
) -> Result<(), RolloutError> {
    use ambientor_mesh::dynamic::{api_resource, list_cr_in_namespace};
    use ambientor_mesh::ingress_collect::{
        build_ingress_context, has_programmed_ambient_ingress, route_uses_sidecar_ingress,
    };

    let hr_ar = api_resource("gateway.networking.k8s.io", "v1", "HTTPRoute", "httproutes");
    let gw_ar = api_resource("gateway.networking.k8s.io", "v1", "Gateway", "gateways");
    let vs_ar = api_resource(
        "networking.istio.io",
        "v1",
        "VirtualService",
        "virtualservices",
    );
    let routes = list_cr_in_namespace(client, &hr_ar, namespace)
        .await
        .map_err(|e| RolloutError::ExecutionFailed(e.to_string()))?;
    let virtual_services = list_cr_in_namespace(client, &vs_ar, namespace)
        .await
        .map_err(|e| RolloutError::ExecutionFailed(e.to_string()))?;
    if routes.is_empty() && virtual_services.is_empty() {
        return Ok(());
    }
    let gateways = list_cr_in_namespace(client, &gw_ar, namespace)
        .await
        .unwrap_or_default();
    // Include cluster-wide ingress gateways referenced by parentRefs.
    let all_gateways = list_cluster_gateways(client, &gw_ar)
        .await
        .unwrap_or(gateways);
    let (_, external_routes) =
        build_ingress_context(&all_gateways, &[], &routes, &virtual_services);
    let public_routes: Vec<_> = external_routes
        .into_iter()
        .filter(|r| r.namespace == namespace)
        .filter(|r| !r.hostnames.is_empty() || r.parent_gateway_name.is_some())
        .collect();
    if public_routes.is_empty() {
        return Ok(());
    }
    let ingress_gateways: Vec<_> = build_ingress_context(&all_gateways, &[], &[], &[])
        .0
        .into_iter()
        .filter(|g| g.gateway_class.as_deref() != Some("istio-waypoint"))
        .collect();
    if !has_programmed_ambient_ingress(&ingress_gateways) {
        return Err(RolloutError::ExecutionFailed(format!(
            "namespace {namespace} has external routes but no programmed ambient ingress Gateway \
             exists; public URLs will break. Create an ambient Gateway in the ingress namespace \
             and update HTTPRoute parentRefs (see assessment finding traffic.ambient-ingress-gateway)"
        )));
    }
    for route in &public_routes {
        if route.parents_attached == Some(false) || route_uses_sidecar_ingress(route, &ingress_gateways)
        {
            let hosts = route.hostnames.join(", ");
            return Err(RolloutError::ExecutionFailed(format!(
                "external route {}/{} (hosts: {hosts}) is not attached to a programmed ambient \
                 ingress Gateway; verify with kubectl get httproute -n {namespace} {} -o yaml",
                route.kind, route.name, route.name
            )));
        }
    }
    Ok(())
}

async fn list_cluster_gateways(
    client: &Client,
    ar: &kube::api::ApiResource,
) -> Result<Vec<kube::api::DynamicObject>, kube::Error> {
    use kube::Api;
    let api = Api::<kube::api::DynamicObject>::all_with(client.clone(), ar);
    Ok(api.list(&kube::api::ListParams::default()).await?.items)
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

/// Confirm application workloads are reachable: Deployments ready, pods running, sidecars removed.
pub async fn verify_application_reachability(
    client: &Client,
    namespace: &str,
) -> Result<(), RolloutError> {
    let deadline = Duration::from_secs(WORKLOAD_READY_TIMEOUT_SECS);
    let started = std::time::Instant::now();
    let mut last_err = format!("workloads in {namespace} not ready");
    while started.elapsed() < deadline {
        match check_application_reachability(client, namespace).await {
            Ok(()) => return Ok(()),
            Err(RolloutError::ExecutionFailed(msg)) => {
                last_err = msg;
                sleep(Duration::from_secs(WORKLOAD_POLL_INTERVAL_SECS)).await;
            }
            Err(e) => return Err(e),
        }
    }
    Err(RolloutError::ExecutionFailed(format!(
        "{last_err} (timed out after {WORKLOAD_READY_TIMEOUT_SECS}s waiting for workloads)"
    )))
}

async fn check_application_reachability(
    client: &Client,
    namespace: &str,
) -> Result<(), RolloutError> {
    use k8s_openapi::api::apps::v1::Deployment;
    use k8s_openapi::api::core::v1::Pod;
    use kube::api::ListParams;
    use kube::Api;

    let dep_api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    let deps = dep_api.list(&ListParams::default()).await?;
    if deps.items.is_empty() {
        return Ok(());
    }

    for dep in &deps.items {
        let name = dep
            .metadata
            .name
            .as_deref()
            .unwrap_or("unknown");
        let desired = dep
            .spec
            .as_ref()
            .and_then(|s| s.replicas)
            .unwrap_or(1);
        let ready = dep
            .status
            .as_ref()
            .and_then(|s| s.ready_replicas)
            .unwrap_or(0);
        if ready < desired {
            return Err(RolloutError::ExecutionFailed(format!(
                "Deployment {namespace}/{name} not ready ({ready}/{desired} replicas)"
            )));
        }
    }

    let pod_api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pods = pod_api.list(&ListParams::default()).await?;
    for pod in &pods.items {
        if is_system_pod(pod) {
            continue;
        }
        let name = pod.metadata.name.as_deref().unwrap_or("unknown");
        let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref());
        if phase != Some("Running") {
            return Err(RolloutError::ExecutionFailed(format!(
                "Pod {namespace}/{name} not Running (phase={phase:?})"
            )));
        }
        let ready = pod
            .status
            .as_ref()
            .and_then(|s| s.conditions.as_ref())
            .is_some_and(|conds| {
                conds
                    .iter()
                    .any(|c| c.type_ == "Ready" && c.status == "True")
            });
        if !ready {
            return Err(RolloutError::ExecutionFailed(format!(
                "Pod {namespace}/{name} not Ready"
            )));
        }
        if pod_has_workload_sidecar(pod) {
            return Err(RolloutError::ExecutionFailed(format!(
                "Pod {namespace}/{name} still has istio-proxy sidecar after migration"
            )));
        }
    }

    Ok(())
}

fn pod_has_workload_sidecar(pod: &k8s_openapi::api::core::v1::Pod) -> bool {
    if pod
        .metadata
        .annotations
        .as_ref()
        .is_some_and(|a| a.contains_key("sidecar.istio.io/status"))
    {
        return true;
    }
    pod.spec.as_ref().is_some_and(|spec| {
        spec.containers
            .iter()
            .any(|c| c.name == "istio-proxy")
            || spec.init_containers.as_ref().is_some_and(|inits| {
                inits
                    .iter()
                    .any(|c| c.name == "istio-proxy" || c.name == "istio-validation")
            })
    })
}

fn is_system_pod(pod: &k8s_openapi::api::core::v1::Pod) -> bool {
    pod.metadata.labels.as_ref().is_some_and(|l| {
        l.get("app")
            .is_some_and(|v| v == "ztunnel" || v.contains("istio"))
            || l.get("istio.io/dataplane-mode") == Some(&"none".into())
    })
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
