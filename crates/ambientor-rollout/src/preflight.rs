use ambientor_mesh::{namespace_conflicts_with_mesh, namespace_enrolled_on_mesh};
use ambientor_types::{MeshInstance, RolloutStage, RolloutStageType};
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

/// True when a later stage will enroll this namespace on the rollout mesh target.
pub fn rollout_will_enroll_namespace(stages: &[RolloutStage], namespace: &str) -> bool {
    stages.iter().any(|s| {
        s.r#type == RolloutStageType::EnrollNamespace && s.namespaces.iter().any(|n| n == namespace)
    })
}

/// True when rollout includes sidecar removal + restart for this namespace.
pub fn rollout_will_remove_sidecars(stages: &[RolloutStage], namespace: &str) -> bool {
    let in_ns = |s: &RolloutStage| s.namespaces.iter().any(|n| n == namespace);
    let has_remove = stages
        .iter()
        .any(|s| s.r#type == RolloutStageType::RemoveInjection && in_ns(s));
    let has_restart = stages
        .iter()
        .any(|s| s.r#type == RolloutStageType::RollingRestart && in_ns(s));
    has_remove && has_restart
}

/// Dry-run only: do not block on demo→ambient enrollment when `EnrollNamespace` is planned.
pub async fn dry_run_namespace(
    client: &Client,
    namespace: &str,
    mesh: &MeshInstance,
    stages: &[RolloutStage],
) -> Result<(), RolloutError> {
    let ns = fetch_namespace(client, namespace).await?;
    let labels = ns.metadata.labels.unwrap_or_default();
    let will_enroll = rollout_will_enroll_namespace(stages, namespace);

    if let Some(msg) = namespace_conflicts_with_mesh(&labels, mesh) {
        if will_enroll {
            return Ok(());
        }
        return Err(RolloutError::ExecutionFailed(format!(
            "namespace {namespace} is on another mesh ({msg}). This rollout has no EnrollNamespace \
             stage — delete the Rollout CR and create a new one from the MigrationPlan (new \
             operator), or enroll manually: ambientor mesh enroll --namespace {namespace} \
             --discovery-label {}",
            mesh.discovery_label
        )));
    }

    if !namespace_enrolled_on_mesh(&labels, mesh) && !will_enroll {
        return Err(RolloutError::ExecutionFailed(format!(
            "namespace {namespace} is not enrolled on mesh '{}' (revision={}). Add an \
             EnrollNamespace stage or enroll before rollout",
            mesh.discovery_label, mesh.enrollment.revision
        )));
    }

    if let Some(pod) = first_sidecar_injected_pod(client, namespace).await?
        && !rollout_will_remove_sidecars(stages, namespace)
    {
        return Err(RolloutError::ExecutionFailed(format!(
            "namespace {namespace} has sidecar-injected pod '{pod}'; this rollout needs \
             RemoveInjection and RollingRestart stages (or remove sidecars manually first)"
        )));
    }

    Ok(())
}

/// Dry-run and early validation before mutating the cluster for ambient migration.
pub async fn preflight_namespace_for_ambient_rollout(
    client: &Client,
    namespace: &str,
    mesh: &MeshInstance,
    stages: &[RolloutStage],
) -> Result<(), RolloutError> {
    if !mesh.ambient {
        return Err(RolloutError::ExecutionFailed(format!(
            "rollout mesh target {} is not an ambient control plane",
            mesh.discovery_label
        )));
    }

    let ns = fetch_namespace(client, namespace).await?;
    let labels = ns.metadata.labels.unwrap_or_default();
    let will_enroll = rollout_will_enroll_namespace(stages, namespace);

    if !namespace_enrolled_on_mesh(&labels, mesh) {
        if let Some(msg) = namespace_conflicts_with_mesh(&labels, mesh) {
            if !will_enroll {
                return Err(RolloutError::ExecutionFailed(format!(
                    "namespace {namespace} is not ready for ambient rollout on mesh '{}': {msg}",
                    mesh.discovery_label
                )));
            }
        } else if !will_enroll {
            return Err(RolloutError::ExecutionFailed(format!(
                "namespace {namespace} is not enrolled on mesh '{}' (revision={}); add an \
                 EnrollNamespace stage or enroll manually",
                mesh.discovery_label, mesh.enrollment.revision
            )));
        }
    }

    if let Some(pod) = first_sidecar_injected_pod(client, namespace).await?
        && !rollout_will_remove_sidecars(stages, namespace)
    {
        let hint = enrollment_hint(mesh);
        return Err(RolloutError::ExecutionFailed(format!(
            "namespace {namespace} still has sidecar-injected pod '{pod}'; {hint}"
        )));
    }

    Ok(())
}

fn enrollment_hint(mesh: &MeshInstance) -> String {
    match (
        mesh.enrollment.discovery_label_key.as_deref(),
        mesh.enrollment.discovery_label_value.as_deref(),
    ) {
        (Some(k), Some(v)) => format!(
            "complete RemoveInjection and RollingRestart stages, or enroll on mesh '{}' ({k}={v}, istio.io/rev={})",
            mesh.discovery_label, mesh.enrollment.revision
        ),
        _ => format!(
            "complete RemoveInjection and RollingRestart stages, or set istio.io/rev={}",
            mesh.enrollment.revision
        ),
    }
}

/// Validates namespace state immediately before creating a waypoint Gateway.
pub async fn preflight_before_deploy_waypoint(
    client: &Client,
    namespace: &str,
    mesh: &MeshInstance,
    stages: &[RolloutStage],
) -> Result<(), RolloutError> {
    preflight_namespace_for_ambient_rollout(client, namespace, mesh, stages).await?;

    let ns = fetch_namespace(client, namespace).await?;
    let labels = ns.metadata.labels.unwrap_or_default();
    let ambient_labeled =
        labels.get("istio.io/dataplane-mode").map(String::as_str) == Some("ambient");
    if !ambient_labeled && !namespace_enrolled_on_mesh(&labels, mesh) {
        let discovery = mesh
            .enrollment
            .discovery_label_key
            .as_deref()
            .zip(mesh.enrollment.discovery_label_value.as_deref())
            .map(|(k, v)| format!("{k}={v}"))
            .unwrap_or_else(|| "revision-only enrollment".into());
        return Err(RolloutError::ExecutionFailed(format!(
            "namespace {namespace} is not enrolled on mesh '{}' ({discovery}, istio.io/rev={}); \
             complete EnrollNamespace and LabelNamespace first",
            mesh.discovery_label, mesh.enrollment.revision
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
        .filter(pod_has_sidecar)
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
            spec.containers.iter().any(|c| c.name == "istio-proxy")
                || spec.init_containers.as_ref().is_some_and(|inits| {
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
    use ambientor_types::{MeshEnrollment, MeshEnrollmentMode, MeshInstance};
    use serde_json::json;

    fn ambient_mesh() -> MeshInstance {
        let enrollment = MeshEnrollment {
            mode: MeshEnrollmentMode::RevisionAndDiscovery,
            revision: "ambient-v1-28-6".into(),
            discovery_label_key: Some("istio-discovery".into()),
            discovery_label_value: Some("mesh-ambient".into()),
            member_roll_namespace: None,
            from_istiod_config: false,
        };
        MeshInstance {
            revision: enrollment.revision.clone(),
            discovery_label: "mesh-ambient".into(),
            control_plane_namespace: "ambient-istio-system".into(),
            version: Some("1.28.6".into()),
            ambient: true,
            enrolled_namespace_count: 2,
            enrollment,
        }
    }

    #[test]
    fn detects_conflicting_discovery_label() {
        use std::collections::BTreeMap;
        let mut labels = BTreeMap::new();
        labels.insert("istio-discovery".into(), "mesh-demo".into());
        labels.insert("istio.io/rev".into(), "demo".into());
        let msg = namespace_conflicts_with_mesh(&labels, &ambient_mesh()).unwrap();
        assert!(msg.contains("demo"));
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
