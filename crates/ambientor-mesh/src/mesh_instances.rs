use std::collections::HashMap;

use ambientor_types::{MeshInstance, MeshTarget};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::api::ListParams;
use kube::{Api, Client};

use crate::mesh_enrollment::build_mesh_enrollment;
use crate::version::parse_revision_version;

pub use crate::mesh_enrollment::{
    enroll_namespace_on_mesh, enrollment_labels_to_apply, read_istiod_discovery_config,
};

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

/// List istiod revisions and resolve enrollment contracts from istiod mesh config + cluster state.
pub async fn discover_mesh_instances(client: &Client) -> anyhow::Result<Vec<MeshInstance>> {
    let flavor_is_ossm = crate::dynamic::list_cluster_cr(
        client,
        &crate::dynamic::api_resource(
            "maistra.io",
            "v1",
            "ServiceMeshMemberRoll",
            "servicemeshmemberrolls",
        ),
    )
    .await
    .map(|rolls| !rolls.is_empty())
    .unwrap_or(false);
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

    let mut instances: Vec<MeshInstance> = Vec::new();
    for (revision, (control_plane_namespace, version, ambient)) in by_revision {
        let discovery_label = infer_discovery_label(&revision, &namespaces);
        let enrolled_namespace_count = discovery_counts.get(&discovery_label).copied().unwrap_or(0);
        let enrollment = build_mesh_enrollment(
            client,
            &revision,
            &control_plane_namespace,
            &discovery_label,
            flavor_is_ossm,
        )
        .await;
        instances.push(MeshInstance {
            revision,
            discovery_label: enrollment
                .discovery_label_value
                .clone()
                .unwrap_or(discovery_label),
            control_plane_namespace,
            version,
            ambient,
            enrolled_namespace_count,
            enrollment,
        });
    }

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

pub use crate::mesh_enrollment::namespace_enrolled_on_mesh as namespace_enrolled_on_mesh_with_enrollment;

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

/// Whether `ns` hosts istiod / mesh control-plane components (not a user application namespace).
pub fn is_istiod_control_plane_namespace(ns: &str, mesh_instances: &[MeshInstance]) -> bool {
    if ns.is_empty() {
        return false;
    }
    if ns == "istio-system" || ns.ends_with("-istio-system") {
        return true;
    }
    mesh_instances
        .iter()
        .any(|m| m.control_plane_namespace == ns)
}

/// User-facing application namespaces eligible for assessment catalog and dashboard rows.
pub fn is_application_namespace(ns: &str, mesh_instances: &[MeshInstance]) -> bool {
    !is_istiod_control_plane_namespace(ns, mesh_instances)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambientor_types::{MeshEnrollment, MeshEnrollmentMode};

    #[test]
    fn excludes_istiod_control_plane_namespaces() {
        let meshes = vec![test_instance(
            "ambient-v1",
            "mesh-ambient",
            "ambient-v1-28-6-istio-system",
            true,
            1,
        )];
        assert!(is_istiod_control_plane_namespace("istio-system", &meshes));
        assert!(is_istiod_control_plane_namespace(
            "ambient-v1-28-6-istio-system",
            &meshes
        ));
        assert!(is_istiod_control_plane_namespace(
            "demo-v1-28-6-istio-system",
            &meshes
        ));
        assert!(!is_istiod_control_plane_namespace("mesh-sidecar-2", &meshes));
        assert!(!is_istiod_control_plane_namespace("bookinfo", &meshes));
    }

    fn test_instance(
        revision: &str,
        discovery_label: &str,
        control_plane_namespace: &str,
        ambient: bool,
        enrolled_namespace_count: usize,
    ) -> MeshInstance {
        let enrollment = MeshEnrollment {
            mode: MeshEnrollmentMode::RevisionAndDiscovery,
            revision: revision.to_string(),
            istio_revision: Some(revision.to_string()),
            revision_tag: None,
            discovery_label_key: Some("istio-discovery".into()),
            discovery_label_value: Some(discovery_label.into()),
            member_roll_namespace: None,
            from_istiod_config: false,
        };
        MeshInstance {
            revision: revision.into(),
            discovery_label: discovery_label.into(),
            control_plane_namespace: control_plane_namespace.into(),
            version: None,
            ambient,
            enrolled_namespace_count,
            enrollment,
        }
    }

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
            test_instance("demo-v1-28-6", "mesh-demo", "demo-istio-system", false, 3),
            test_instance(
                "ambient-v1-28-6",
                "mesh-ambient",
                "ambient-istio-system",
                true,
                2,
            ),
        ];
        let picked = resolve_mesh_target(&instances, None).unwrap();
        assert_eq!(picked.discovery_label, "mesh-ambient");
    }

    #[test]
    fn ambiguous_without_target() {
        let instances = vec![
            test_instance("ambient-a", "mesh-ambient-a", "a-istio-system", true, 0),
            test_instance("ambient-b", "mesh-ambient-b", "b-istio-system", true, 0),
        ];
        assert!(matches!(
            resolve_mesh_target(&instances, None),
            Err(MeshTargetError::Ambiguous { .. })
        ));
    }

    #[test]
    fn explicit_discovery_label() {
        let instances = vec![
            test_instance(
                "ambient-v1-28-6",
                "mesh-ambient",
                "ambient-istio-system",
                true,
                1,
            ),
            test_instance("demo-v1-28-6", "mesh-demo", "demo-istio-system", false, 3),
        ];
        let sel = MeshTarget {
            discovery_label: Some("mesh-demo".into()),
            ..Default::default()
        };
        let picked = resolve_mesh_target(&instances, Some(&sel)).unwrap();
        assert_eq!(picked.discovery_label, "mesh-demo");
    }
}
