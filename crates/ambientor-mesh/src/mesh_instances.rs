use std::collections::HashMap;

use ambientor_types::{MeshInstance, MeshTarget};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::api::ListParams;
use kube::{Api, Client};

use crate::version::parse_revision_version;

#[derive(Debug, thiserror::Error)]
pub enum MeshTargetError {
    #[error("no ambient Istio control plane found in the cluster")]
    NoAmbientMesh,
    #[error(
        "multiple ambient mesh instances found ({list}); set rollout.spec.meshTarget to one of them"
    )]
    Ambiguous { list: String },
    #[error("meshTarget does not match any discovered mesh instance")]
    NotFound,
}

/// List istiod revisions and infer `istio-discovery` label values from enrolled namespaces.
pub async fn discover_mesh_instances(client: &Client) -> anyhow::Result<Vec<MeshInstance>> {
    let namespaces = list_namespaces(client).await?;
    let discovery_counts = discovery_label_counts(&namespaces);
    let ztunnel_revs = ztunnel_revisions(client).await.unwrap_or_default();
    let mut by_revision: HashMap<String, (String, Option<String>, bool)> = HashMap::new();

    for cp_ns in istiod_namespace_candidates(client).await {
        let api: Api<Deployment> = Api::namespaced(client.clone(), &cp_ns);
        let deployments = api.list(&ListParams::default()).await?;
        for dep in deployments.items {
            let name = dep.metadata.name.as_deref().unwrap_or("");
            if !name.contains("istiod") {
                continue;
            }
            let revision = istio_revision_from_deployment(&dep);
            let Some(revision) = revision else {
                continue;
            };
            let version = parse_revision_version(&revision);
            // OSSM/Sail: ambient revisions/namespaces contain "ambient".
            // Upstream Istio ambient: revision may be "default"; detect ambient via ztunnel presence.
            let ambient = revision.to_ascii_lowercase().contains("ambient")
                || cp_ns.to_ascii_lowercase().contains("ambient")
                || (!ztunnel_revs.is_empty()
                    && (ztunnel_revs.contains(&revision)
                        || (ztunnel_revs.len() == 1 && revision == "default")));
            by_revision
                .entry(revision.clone())
                .or_insert_with(|| (cp_ns.clone(), version, ambient));
        }
    }

    let mut instances: Vec<MeshInstance> = by_revision
        .into_iter()
        .map(|(revision, (control_plane_namespace, version, ambient))| {
            let discovery_label = infer_discovery_label(&revision, &namespaces);
            let enrolled_namespace_count =
                discovery_counts.get(&discovery_label).copied().unwrap_or(0);
            MeshInstance {
                revision,
                discovery_label,
                control_plane_namespace,
                version,
                ambient,
                enrolled_namespace_count,
            }
        })
        .collect();

    instances.sort_by(|a, b| {
        a.discovery_label
            .cmp(&b.discovery_label)
            .then(a.revision.cmp(&b.revision))
    });

    // Upstream Istio ambient profile in kind often uses the default revision. If ztunnel exists and
    // only one istiod revision was discovered, treat it as ambient so rollouts can proceed.
    if !ztunnel_revs.is_empty() && instances.len() == 1 {
        instances[0].ambient = true;
    }

    Ok(instances)
}

/// Pick the mesh instance for a rollout: explicit `meshTarget`, or auto when exactly one ambient mesh exists.
pub fn resolve_mesh_target(
    instances: &[MeshInstance],
    target: Option<&MeshTarget>,
) -> Result<MeshInstance, MeshTargetError> {
    if let Some(sel) = target {
        return instances
            .iter()
            .find(|i| mesh_target_matches(sel, i))
            .cloned()
            .ok_or(MeshTargetError::NotFound);
    }

    let ambient: Vec<_> = instances.iter().filter(|i| i.ambient).collect();
    match ambient.len() {
        0 => Err(MeshTargetError::NoAmbientMesh),
        1 => Ok(ambient[0].clone()),
        n => Err(MeshTargetError::Ambiguous {
            list: ambient[..n]
                .iter()
                .map(|i| {
                    format!(
                        "{} (revision={}, ns={})",
                        i.discovery_label, i.revision, i.control_plane_namespace
                    )
                })
                .collect::<Vec<_>>()
                .join(", "),
        }),
    }
}

pub fn mesh_target_matches(sel: &MeshTarget, instance: &MeshInstance) -> bool {
    if let Some(ref rev) = sel.revision
        && rev == &instance.revision
    {
        return true;
    }
    if let Some(ref label) = sel.discovery_label
        && label == &instance.discovery_label
    {
        return true;
    }
    if let Some(ref ns) = sel.control_plane_namespace
        && ns == &instance.control_plane_namespace
    {
        return true;
    }
    false
}

/// True when the namespace is enrolled on this mesh instance (discovery selector / revision).
pub fn namespace_enrolled_on_mesh(
    labels: &std::collections::BTreeMap<String, String>,
    mesh: &MeshInstance,
) -> bool {
    if labels.get("istio-discovery").map(String::as_str) == Some(mesh.discovery_label.as_str()) {
        return true;
    }
    if labels.get("istio.io/rev").map(String::as_str) == Some(mesh.revision.as_str()) {
        return true;
    }
    false
}

async fn list_namespaces(client: &Client) -> anyhow::Result<Vec<Namespace>> {
    let api: Api<Namespace> = Api::all(client.clone());
    Ok(api.list(&ListParams::default()).await?.items)
}

async fn ztunnel_revisions(client: &Client) -> anyhow::Result<Vec<String>> {
    let api: Api<Pod> = Api::all(client.clone());
    let pods = api
        .list(&ListParams::default().labels("app=ztunnel"))
        .await?;
    let mut set = std::collections::BTreeSet::new();
    for p in pods.items {
        if let Some(rev) = p
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("istio.io/rev"))
        {
            set.insert(rev.clone());
        }
    }
    Ok(set.into_iter().collect())
}

fn discovery_label_counts(namespaces: &[Namespace]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for ns in namespaces {
        if let Some(label) = ns
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("istio-discovery"))
        {
            *counts.entry(label.clone()).or_insert(0) += 1;
        }
    }
    counts
}

async fn istiod_namespace_candidates(client: &Client) -> Vec<String> {
    let mut namespaces = vec!["istio-system".to_string()];
    if let Ok(all) = list_namespaces(client).await {
        for ns in all {
            if let Some(name) = ns.metadata.name
                && name.ends_with("-istio-system")
                && !namespaces.iter().any(|n| n == &name)
            {
                namespaces.push(name);
            }
        }
    }
    namespaces
}

fn istio_revision_from_deployment(dep: &Deployment) -> Option<String> {
    dep.metadata
        .labels
        .as_ref()
        .and_then(|l| l.get("istio.io/rev").cloned())
        .or_else(|| {
            dep.spec
                .as_ref()
                .and_then(|s| s.template.metadata.as_ref())
                .and_then(|m| m.labels.as_ref())
                .and_then(|l| l.get("istio.io/rev").cloned())
        })
        .or_else(|| {
            dep.metadata
                .name
                .as_deref()
                .and_then(|n| n.strip_prefix("istiod-").map(str::to_string))
        })
}

/// Prefer the discovery label already used by namespaces on this revision; else `mesh-{short}`.
pub fn infer_discovery_label(revision: &str, namespaces: &[Namespace]) -> String {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for ns in namespaces {
        let labels = match ns.metadata.labels.as_ref() {
            Some(l) => l,
            None => continue,
        };
        let rev_match = labels.get("istio.io/rev").map(String::as_str) == Some(revision);
        if rev_match && let Some(d) = labels.get("istio-discovery") {
            *counts.entry(d.clone()).or_insert(0) += 1;
        }
    }
    if counts.len() == 1 {
        return counts.into_keys().next().expect("single key");
    }
    let mut best: Option<(usize, String)> = None;
    for (label, count) in counts {
        if best.as_ref().is_none_or(|(c, _)| count > *c) {
            best = Some((count, label));
        }
    }
    if let Some((_, label)) = best
        && !label.is_empty()
    {
        return label;
    }
    format!("mesh-{}", revision_short_name(revision))
}

fn revision_short_name(revision: &str) -> &str {
    revision.split("-v").next().unwrap_or(revision)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(labels: Vec<(&str, &str)>) -> Namespace {
        Namespace {
            metadata: kube::api::ObjectMeta {
                labels: Some(
                    labels
                        .into_iter()
                        .map(|(k, v)| (k.into(), v.into()))
                        .collect(),
                ),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn infers_discovery_from_namespace_labels() {
        let namespaces = vec![
            ns(vec![
                ("istio.io/rev", "ambient-v1-28-6"),
                ("istio-discovery", "mesh-ambient"),
            ]),
            ns(vec![
                ("istio.io/rev", "ambient-v1-28-6"),
                ("istio-discovery", "mesh-ambient"),
            ]),
        ];
        assert_eq!(
            infer_discovery_label("ambient-v1-28-6", &namespaces),
            "mesh-ambient"
        );
    }

    #[test]
    fn infers_mesh_prefix_when_no_namespaces() {
        assert_eq!(
            infer_discovery_label("ambient-v1-28-6", &[]),
            "mesh-ambient"
        );
    }

    #[test]
    fn auto_select_single_ambient() {
        let instances = vec![
            MeshInstance {
                revision: "demo-v1-28-6".into(),
                discovery_label: "mesh-demo".into(),
                control_plane_namespace: "demo-istio-system".into(),
                version: Some("1.28.6".into()),
                ambient: false,
                enrolled_namespace_count: 3,
            },
            MeshInstance {
                revision: "ambient-v1-28-6".into(),
                discovery_label: "mesh-ambient".into(),
                control_plane_namespace: "ambient-istio-system".into(),
                version: Some("1.28.6".into()),
                ambient: true,
                enrolled_namespace_count: 2,
            },
        ];
        let picked = resolve_mesh_target(&instances, None).unwrap();
        assert_eq!(picked.discovery_label, "mesh-ambient");
    }

    #[test]
    fn ambiguous_without_target() {
        let instances = vec![
            MeshInstance {
                revision: "ambient-a".into(),
                discovery_label: "mesh-ambient-a".into(),
                control_plane_namespace: "a-istio-system".into(),
                version: None,
                ambient: true,
                enrolled_namespace_count: 0,
            },
            MeshInstance {
                revision: "ambient-b".into(),
                discovery_label: "mesh-ambient-b".into(),
                control_plane_namespace: "b-istio-system".into(),
                version: None,
                ambient: true,
                enrolled_namespace_count: 0,
            },
        ];
        assert!(matches!(
            resolve_mesh_target(&instances, None),
            Err(MeshTargetError::Ambiguous { .. })
        ));
    }

    #[test]
    fn explicit_discovery_label() {
        let instances = vec![
            MeshInstance {
                revision: "ambient-v1-28-6".into(),
                discovery_label: "mesh-ambient".into(),
                control_plane_namespace: "ambient-istio-system".into(),
                version: None,
                ambient: true,
                enrolled_namespace_count: 1,
            },
            MeshInstance {
                revision: "demo-v1-28-6".into(),
                discovery_label: "mesh-demo".into(),
                control_plane_namespace: "demo-istio-system".into(),
                version: None,
                ambient: false,
                enrolled_namespace_count: 3,
            },
        ];
        let sel = MeshTarget {
            discovery_label: Some("mesh-demo".into()),
            ..Default::default()
        };
        let picked = resolve_mesh_target(&instances, Some(&sel)).unwrap();
        assert_eq!(picked.discovery_label, "mesh-demo");
    }
}
