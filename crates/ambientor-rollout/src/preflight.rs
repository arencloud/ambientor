use std::collections::BTreeMap;

use ambientor_mesh::mesh_instances::namespace_enrolled_on_mesh;
use ambientor_types::MeshInstance;
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::api::ListParams;
use kube::{Api, Client};
use serde_json::Value;

use crate::engine::RolloutError;
use crate::verify::gateway_ready;
use crate::waypoint::WAYPOINT_GATEWAY_NAME;

/// Namespaces that appear in any stage of the rollout spec.
pub fn namespaces_in_rollout(stages: &[ambientor_types::RolloutStage]) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    for stage in stages {
        for ns in &stage.namespaces {
            if !ns.is_empty() {
                set.insert(ns.clone());
            }
        }
    }
    set.into_iter().collect()
}

/// Dry-run and early validation before mutating the cluster for ambient migration.
pub async fn preflight_namespace_for_ambient_rollout(
    client: &Client,
    namespace: &str,
    mesh: &MeshInstance,
) -> Result<(), RolloutError> {
    if !mesh.ambient {
        return Err(RolloutError::ExecutionFailed(format!(
            "rollout mesh target {} is not an ambient control plane",
            mesh.discovery_label
        )));
    }

    let ns = fetch_namespace(client, namespace).await?;
    let labels = ns.metadata.labels.unwrap_or_default();

    if namespace_enrolled_on_mesh(&labels, mesh) {
        return Ok(());
    }

    if let Some(msg) = namespace_conflicts_with_mesh(&labels, mesh) {
        return Err(RolloutError::ExecutionFailed(format!(
            "namespace {namespace} is not ready for ambient rollout on mesh '{}': {msg}",
            mesh.discovery_label
        )));
    }

    if let Some(pod) = first_sidecar_injected_pod(client, namespace).await? {
        return Err(RolloutError::ExecutionFailed(format!(
            "namespace {namespace} still has sidecar-injected pod '{pod}'; enroll it on mesh '{}' \
             (istio-discovery={}) before rollout",
            mesh.discovery_label, mesh.discovery_label
        )));
    }

    Ok(())
}

/// Validates namespace state immediately before creating a waypoint Gateway.
pub async fn preflight_before_deploy_waypoint(
    client: &Client,
    namespace: &str,
    mesh: &MeshInstance,
) -> Result<(), RolloutError> {
    preflight_namespace_for_ambient_rollout(client, namespace, mesh).await?;

    let ns = fetch_namespace(client, namespace).await?;
    let labels = ns.metadata.labels.unwrap_or_default();
    let ambient_labeled = labels.get("istio.io/dataplane-mode").map(String::as_str) == Some("ambient");
    if !ambient_labeled && !namespace_enrolled_on_mesh(&labels, mesh) {
        return Err(RolloutError::ExecutionFailed(format!(
            "namespace {namespace} is not enrolled on mesh '{}' (expected istio-discovery={} or \
             istio.io/rev={}); complete LabelNamespace or OSSM discovery first",
            mesh.discovery_label, mesh.discovery_label, mesh.revision
        )));
    }

    Ok(())
}

/// Human-readable hint when a waypoint Gateway never leaves "Waiting for controller".
pub fn waypoint_gateway_stuck_message(
    namespace: &str,
    mesh: &MeshInstance,
    gateway_status: Option<&Value>,
) -> String {
    let mut msg = format!(
        "waypoint Gateway {WAYPOINT_GATEWAY_NAME} in {namespace} was not programmed within the timeout \
         (mesh discovery={}, revision={})",
        mesh.discovery_label, mesh.revision
    );
    if gateway_controller_pending(gateway_status) {
        msg.push_str(
            ". Gateway status is still 'Waiting for controller' — the namespace is likely not \
             managed by this mesh instance's istiod",
        );
    }
    msg
}

/// Returns true when Gateway status shows the controller has not accepted the resource yet.
pub fn gateway_controller_pending(gateway_status: Option<&Value>) -> bool {
    let Some(data) = gateway_status else {
        return false;
    };
    if gateway_ready(data) {
        return false;
    }
    data.get("status")
        .and_then(|s| s.get("conditions"))
        .and_then(|c| c.as_array())
        .is_some_and(|conds| {
            conds.iter().any(|c| {
                c.get("reason").and_then(|r| r.as_str()) == Some("Pending")
                    && c.get("message")
                        .and_then(|m| m.as_str())
                        .is_some_and(|m| m.contains("Waiting for controller"))
            })
        })
}

/// Namespace is on a different Istio discovery / revision than the rollout target.
pub fn namespace_conflicts_with_mesh(
    labels: &BTreeMap<String, String>,
    mesh: &MeshInstance,
) -> Option<String> {
    if let Some(discovery) = labels.get("istio-discovery") {
        if discovery != &mesh.discovery_label {
            return Some(format!(
                "namespace has istio-discovery={discovery}, expected {}",
                mesh.discovery_label
            ));
        }
    }
    if let Some(rev) = labels.get("istio.io/rev") {
        if rev != &mesh.revision {
            return Some(format!(
                "namespace has istio.io/rev={rev}, expected {}",
                mesh.revision
            ));
        }
    }
    None
}

async fn fetch_namespace(client: &Client, name: &str) -> Result<Namespace, RolloutError> {
    let api: Api<Namespace> = Api::all(client.clone());
    api.get(name).await.map_err(RolloutError::Kube)
}

async fn first_sidecar_injected_pod(
    client: &Client,
    namespace: &str,
) -> Result<Option<String>, RolloutError> {
    let api: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let pods = api.list(&ListParams::default()).await?;
    Ok(pods
        .items
        .into_iter()
        .filter(|p| pod_has_sidecar(p))
        .filter_map(|p| p.metadata.name)
        .next())
}

fn pod_has_sidecar(pod: &Pod) -> bool {
    if pod
        .metadata
        .annotations
        .as_ref()
        .is_some_and(|a| a.contains_key("sidecar.istio.io/status"))
    {
        return true;
    }
    pod.spec
        .as_ref()
        .map(|spec| {
            spec.containers
                .iter()
                .any(|c| c.name == "istio-proxy")
                || spec
                    .init_containers
                    .as_ref()
                    .is_some_and(|inits| {
                        inits
                            .iter()
                            .any(|c| c.name == "istio-proxy" || c.name == "istio-validation")
                    })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambientor_types::MeshInstance;
    use serde_json::json;

    fn ambient_mesh() -> MeshInstance {
        MeshInstance {
            revision: "ambient-v1-28-6".into(),
            discovery_label: "mesh-ambient".into(),
            control_plane_namespace: "ambient-istio-system".into(),
            version: Some("1.28.6".into()),
            ambient: true,
            enrolled_namespace_count: 2,
        }
    }

    #[test]
    fn detects_conflicting_discovery_label() {
        let mut labels = BTreeMap::new();
        labels.insert("istio-discovery".into(), "mesh-demo".into());
        let msg = namespace_conflicts_with_mesh(&labels, &ambient_mesh()).unwrap();
        assert!(msg.contains("mesh-demo"));
    }

    #[test]
    fn detects_pending_gateway_controller() {
        let status = json!({
            "status": {
                "conditions": [{
                    "type": "Programmed",
                    "status": "Unknown",
                    "reason": "Pending",
                    "message": "Waiting for controller"
                }]
            }
        });
        assert!(gateway_controller_pending(Some(&status)));
    }
}
