//! Infer logical application identity from live pod labels (not namespace name alone).

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Pod;

/// Application identity for a namespace, derived from workload pods.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamespaceApplicationIdentity {
    /// Primary display name (e.g. `bookinfo`, `reviews`).
    pub application_name: String,
    /// Distinct workload / component names in this namespace.
    pub workload_components: Vec<String>,
    /// How `application_name` was chosen (`app.kubernetes.io/name`, `app`, pod owner, …).
    pub name_source: String,
    /// User-facing pods counted as application workloads (excludes mesh infra).
    pub app_pod_count: u32,
}

/// Build a map of namespace → application identity from cluster pods.
pub fn identities_by_namespace(pods: &[Pod]) -> BTreeMap<String, NamespaceApplicationIdentity> {
    let mut by_ns: BTreeMap<String, Vec<&Pod>> = BTreeMap::new();
    for pod in pods {
        let Some(ns) = pod.metadata.namespace.as_ref() else {
            continue;
        };
        if is_mesh_infrastructure_pod(pod) {
            continue;
        }
        if pod_is_terminal(pod) {
            continue;
        }
        by_ns.entry(ns.clone()).or_default().push(pod);
    }

    by_ns
        .into_iter()
        .map(|(ns, pods)| (ns.clone(), infer_from_pods(&ns, &pods)))
        .collect()
}

pub fn infer_from_pods(namespace: &str, pods: &[&Pod]) -> NamespaceApplicationIdentity {
    if pods.is_empty() {
        return NamespaceApplicationIdentity {
            application_name: namespace.to_string(),
            workload_components: vec![],
            name_source: "namespace".into(),
            app_pod_count: 0,
        };
    }

    let mut name_votes: BTreeMap<String, u32> = BTreeMap::new();
    let mut name_source = "namespace".to_string();
    let mut components: BTreeMap<String, ()> = BTreeMap::new();

    for pod in pods {
        if let Some((name, source)) = pod_logical_name(pod) {
            *name_votes.entry(name.clone()).or_default() += 1;
            components.insert(name, ());
            if source != "pod" {
                name_source = source.to_string();
            }
        }
    }

    let application_name = name_votes
        .iter()
        .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
        .map(|(n, _)| n.clone())
        .unwrap_or_else(|| namespace.to_string());

    let mut workload_components: Vec<_> = components.into_keys().collect();
    workload_components.sort();

    if workload_components.is_empty() {
        workload_components.push(application_name.clone());
    }

    NamespaceApplicationIdentity {
        application_name,
        workload_components,
        name_source,
        app_pod_count: pods.len() as u32,
    }
}

fn pod_logical_name(pod: &Pod) -> Option<(String, &'static str)> {
    let labels = pod.metadata.labels.as_ref()?;
    if let Some(v) = labels.get("app.kubernetes.io/name") {
        return Some((v.clone(), "app.kubernetes.io/name"));
    }
    if let Some(v) = labels.get("app") {
        return Some((v.clone(), "app"));
    }
    if let Some(v) = labels.get("app.kubernetes.io/instance") {
        return Some((v.clone(), "app.kubernetes.io/instance"));
    }
    if let Some(v) = labels.get("k8s-app") {
        return Some((v.clone(), "k8s-app"));
    }
    pod.metadata.name.as_ref().map(|n| {
        (
            strip_pod_suffix(n),
            "pod",
        )
    })
}

/// Strip ReplicaSet-style hash suffix (`reviews-v1-7b8c9d`) → `reviews-v1`.
fn strip_pod_suffix(name: &str) -> String {
    let parts: Vec<&str> = name.split('-').collect();
    if parts.len() >= 3 {
        let last = parts[parts.len() - 1];
        let prev = parts[parts.len() - 2];
        if last.len() >= 5
            && last.chars().all(|c| c.is_ascii_alphanumeric())
            && prev.chars().any(|c| c.is_ascii_digit())
        {
            return parts[..parts.len() - 1].join("-");
        }
    }
    name.to_string()
}

/// Workload / application names that identify mesh control-plane or dataplane infra (not user apps).
pub fn is_mesh_infrastructure_workload_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "ztunnel"
            | "istio-ztunnel"
            | "istiod"
            | "istio-ingressgateway"
            | "istio-egressgateway"
            | "istio-cni-node"
            | "waypoint"
    ) || lower.starts_with("istiod-")
        || lower.starts_with("ztunnel-")
        || lower.starts_with("istio-ingressgateway-")
        || lower.starts_with("istio-egressgateway-")
}

/// Count running user-facing pods in a namespace (mesh infra and terminal pods excluded).
pub fn application_pod_count_for_namespace(pods: &[Pod], namespace: &str) -> u32 {
    pods.iter()
        .filter(|p| p.metadata.namespace.as_deref() == Some(namespace))
        .filter(|p| !is_mesh_infrastructure_pod(p) && !pod_is_terminal(p))
        .count() as u32
}

/// True when every counted pod in the namespace is mesh infrastructure (e.g. only ztunnel).
pub fn is_mesh_infrastructure_identity(id: &NamespaceApplicationIdentity) -> bool {
    if id.app_pod_count == 0 {
        return false;
    }
    is_mesh_infrastructure_workload_name(&id.application_name)
        || id.workload_components
            .iter()
            .all(|c| is_mesh_infrastructure_workload_name(c))
}

fn is_mesh_infrastructure_pod(pod: &Pod) -> bool {
    if pod
        .metadata
        .name
        .as_deref()
        .is_some_and(|n| is_mesh_infrastructure_workload_name(n))
    {
        return true;
    }
    let labels = match pod.metadata.labels.as_ref() {
        Some(l) => l,
        None => return false,
    };
    if labels
        .get("app")
        .is_some_and(|v| is_mesh_infrastructure_workload_name(v))
    {
        return true;
    }
    if labels
        .get("app.kubernetes.io/name")
        .is_some_and(|v| is_mesh_infrastructure_workload_name(v))
    {
        return true;
    }
    if labels
        .get("app.kubernetes.io/component")
        .is_some_and(|v| is_mesh_infrastructure_workload_name(v))
    {
        return true;
    }
    if labels
        .get("istio")
        .is_some_and(|v| v.contains("ingress") || v == "ingressgateway")
    {
        return true;
    }
    pod.spec.as_ref().is_some_and(|spec| {
        spec.containers.len() == 1
            && spec.containers.iter().all(|c| {
                c.name == "istio-proxy" || c.name == "ztunnel" || c.name == "waypoint"
            })
    })
}

fn pod_is_terminal(pod: &Pod) -> bool {
    pod.status.as_ref().is_some_and(|s| {
        s.phase.as_deref() == Some("Succeeded") || s.phase.as_deref() == Some("Failed")
    })
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::{Container, Pod, PodSpec};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    use super::*;

    fn pod(ns: &str, name: &str, labels: Vec<(&str, &str)>) -> Pod {
        Pod {
            metadata: ObjectMeta {
                namespace: Some(ns.into()),
                name: Some(name.into()),
                labels: Some(labels.into_iter().map(|(k, v)| (k.into(), v.into())).collect()),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "app".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn prefers_app_kubernetes_io_name() {
        let pods = vec![pod(
            "bookinfo",
            "reviews-v1-abc12",
            vec![("app.kubernetes.io/name", "reviews"), ("app", "reviews-v1")],
        )];
        let refs: Vec<_> = pods.iter().collect();
        let id = infer_from_pods("bookinfo", &refs);
        assert_eq!(id.application_name, "reviews");
        assert_eq!(id.name_source, "app.kubernetes.io/name");
    }

    #[test]
    fn skips_ztunnel() {
        let pods = vec![
            pod("istio-system", "ztunnel-abc", vec![("app", "ztunnel")]),
            pod("bookinfo", "product-v1-x", vec![("app", "product")]),
        ];
        let map = identities_by_namespace(&pods);
        assert!(!map.contains_key("istio-system"));
        assert_eq!(map["bookinfo"].application_name, "product");
    }

    #[test]
    fn skips_ztunnel_by_app_kubernetes_io_name() {
        let pods = vec![pod(
            "ambient-v1-28-6-istio-system",
            "ztunnel-abc",
            vec![("app.kubernetes.io/name", "ztunnel")],
        )];
        assert!(identities_by_namespace(&pods).is_empty());
    }

    #[test]
    fn detects_infra_identity() {
        let id = NamespaceApplicationIdentity {
            application_name: "ztunnel".into(),
            workload_components: vec!["ztunnel".into()],
            name_source: "app".into(),
            app_pod_count: 3,
        };
        assert!(is_mesh_infrastructure_identity(&id));
    }
}
