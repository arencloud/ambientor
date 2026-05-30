use std::collections::{BTreeMap, BTreeSet};

use ambientor_types::{MeshEnrollment, MeshEnrollmentMode, MeshInstance};
use k8s_openapi::api::core::v1::ConfigMap;
use kube::api::{Patch, PatchParams};
use kube::{Api, Client};

use crate::dynamic::{api_resource, list_cr_in_namespace};
use crate::platform_scan::collect_ossm_member_namespaces;
use crate::revision_tags::preferred_namespace_revision_label;

const DEFAULT_DISCOVERY_KEY: &str = "istio-discovery";
const REVISION_LABEL: &str = "istio.io/rev";

/// Parsed `mesh` ConfigMap discovery selector for one istiod revision.
#[derive(Clone, Debug, Default)]
pub struct IstiodDiscoveryConfig {
    pub revision: Option<String>,
    pub discovery_label_key: Option<String>,
    pub discovery_label_value: Option<String>,
    pub from_istiod_config: bool,
}

/// Read istiod mesh config for a control-plane namespace + revision.
pub async fn read_istiod_discovery_config(
    client: &Client,
    control_plane_namespace: &str,
    revision: &str,
) -> IstiodDiscoveryConfig {
    let api: Api<ConfigMap> = Api::namespaced(client.clone(), control_plane_namespace);
    let candidates = [
        format!("istio-{revision}"),
        "istio".to_string(),
        format!("istio-sidecar-injector-{revision}"),
    ];
    for name in candidates {
        let Ok(cm) = api.get(&name).await else {
            continue;
        };
        let Some(data) = cm.data.as_ref() else {
            continue;
        };
        let Some(mesh_yaml) = data.get("mesh") else {
            continue;
        };
        if let Some(parsed) = parse_mesh_config_discovery(mesh_yaml) {
            return parsed;
        }
    }
    IstiodDiscoveryConfig::default()
}

fn parse_mesh_config_discovery(mesh_yaml: &str) -> Option<IstiodDiscoveryConfig> {
    let root: serde_json::Value = serde_yaml::from_str(mesh_yaml).ok()?;
    let selectors = root.get("discoverySelectors")?.as_array()?;
    let mut revision = None;
    let mut discovery_key = None;
    let mut discovery_value = None;

    for sel in selectors {
        let labels = sel.get("matchLabels")?.as_object()?;
        if let Some(rev) = labels.get(REVISION_LABEL).and_then(|v| v.as_str()) {
            revision = Some(rev.to_string());
        }
        for (k, v) in labels {
            if k == REVISION_LABEL {
                continue;
            }
            if let Some(val) = v.as_str() {
                discovery_key = Some(k.clone());
                discovery_value = Some(val.to_string());
            }
        }
    }

    if revision.is_none() && discovery_key.is_none() {
        return None;
    }

    Some(IstiodDiscoveryConfig {
        revision,
        discovery_label_key: discovery_key,
        discovery_label_value: discovery_value,
        from_istiod_config: true,
    })
}

/// Build enrollment contract for a discovered mesh instance.
pub async fn build_mesh_enrollment(
    client: &Client,
    revision: &str,
    control_plane_namespace: &str,
    inferred_discovery_value: &str,
    flavor_is_ossm: bool,
) -> MeshEnrollment {
    let istiod = read_istiod_discovery_config(client, control_plane_namespace, revision).await;
    let member_roll_in_cp = ossm_member_roll_exists(client, control_plane_namespace).await;

    let (discovery_key, discovery_value) = if let (Some(k), Some(v)) = (
        istiod.discovery_label_key.as_ref(),
        istiod.discovery_label_value.as_ref(),
    ) {
        (Some(k.clone()), Some(v.clone()))
    } else if !inferred_discovery_value.is_empty() {
        (
            Some(DEFAULT_DISCOVERY_KEY.into()),
            Some(inferred_discovery_value.to_string()),
        )
    } else {
        (None, None)
    };

    let istio_revision = istiod.revision.unwrap_or_else(|| revision.to_string());
    let (namespace_rev_label, revision_tag) =
        preferred_namespace_revision_label(client, control_plane_namespace, &istio_revision).await;

    let mode = if flavor_is_ossm && member_roll_in_cp {
        MeshEnrollmentMode::OssmMemberRoll
    } else if discovery_key.is_some() && discovery_value.is_some() {
        MeshEnrollmentMode::RevisionAndDiscovery
    } else if discovery_key.is_some() {
        MeshEnrollmentMode::DiscoveryLabel
    } else {
        MeshEnrollmentMode::RevisionOnly
    };

    MeshEnrollment {
        mode,
        revision: namespace_rev_label,
        istio_revision: Some(istio_revision),
        revision_tag,
        discovery_label_key: discovery_key,
        discovery_label_value: discovery_value,
        member_roll_namespace: if member_roll_in_cp {
            Some(control_plane_namespace.to_string())
        } else {
            None
        },
        from_istiod_config: istiod.from_istiod_config,
    }
}

async fn ossm_member_roll_exists(client: &Client, control_plane_namespace: &str) -> bool {
    let ar = api_resource(
        "maistra.io",
        "v1",
        "ServiceMeshMemberRoll",
        "servicemeshmemberrolls",
    );
    list_cr_in_namespace(client, &ar, control_plane_namespace)
        .await
        .map(|rolls| !rolls.is_empty())
        .unwrap_or(false)
}

/// Whether namespace labels satisfy this mesh instance's enrollment contract.
pub fn namespace_enrolled_on_mesh(labels: &BTreeMap<String, String>, mesh: &MeshInstance) -> bool {
    let e = &mesh.enrollment;
    match e.mode {
        MeshEnrollmentMode::RevisionOnly => {
            labels.get(REVISION_LABEL).map(String::as_str) == Some(e.revision.as_str())
        }
        MeshEnrollmentMode::DiscoveryLabel => discovery_matches(labels, e),
        MeshEnrollmentMode::RevisionAndDiscovery | MeshEnrollmentMode::OssmMemberRoll => {
            revision_matches(labels, e) && discovery_matches(labels, e)
        }
    }
}

fn revision_matches(labels: &BTreeMap<String, String>, e: &MeshEnrollment) -> bool {
    let ns_rev = labels.get(REVISION_LABEL).map(String::as_str);
    ns_rev == Some(e.revision.as_str())
        || e.istio_revision
            .as_ref()
            .is_some_and(|r| ns_rev == Some(r.as_str()))
        || e.revision_tag
            .as_ref()
            .is_some_and(|t| ns_rev == Some(t.as_str()))
}

fn discovery_matches(labels: &BTreeMap<String, String>, e: &MeshEnrollment) -> bool {
    match (&e.discovery_label_key, &e.discovery_label_value) {
        (Some(k), Some(v)) => labels.get(k).map(String::as_str) == Some(v.as_str()),
        _ => true,
    }
}

/// Namespace carries a different revision or discovery selector than the rollout target.
pub fn namespace_conflicts_with_mesh(
    labels: &BTreeMap<String, String>,
    mesh: &MeshInstance,
) -> Option<String> {
    let e = &mesh.enrollment;
    if let Some(rev) = labels.get(REVISION_LABEL)
        && rev != &e.revision
    {
        return Some(format!(
            "namespace has {REVISION_LABEL}={rev}, expected {}",
            e.revision
        ));
    }
    if let (Some(k), Some(v)) = (&e.discovery_label_key, &e.discovery_label_value)
        && let Some(actual) = labels.get(k)
        && actual != v
    {
        return Some(format!("namespace has {k}={actual}, expected {v}"));
    }
    None
}

/// Labels Ambientor applies when enrolling a namespace on this mesh instance.
pub fn enrollment_labels_to_apply(mesh: &MeshInstance) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    out.insert(REVISION_LABEL.into(), mesh.enrollment.revision.clone());
    if let (Some(k), Some(v)) = (
        mesh.enrollment.discovery_label_key.as_ref(),
        mesh.enrollment.discovery_label_value.as_ref(),
    ) {
        out.insert(k.clone(), v.clone());
    }
    out
}

/// Enroll namespace: OSSM MemberRoll (when required) + discovery/revision labels.
pub async fn enroll_namespace_on_mesh(
    client: &Client,
    namespace: &str,
    mesh: &MeshInstance,
) -> anyhow::Result<Vec<String>> {
    let mut actions = Vec::new();
    if mesh.enrollment.mode == MeshEnrollmentMode::OssmMemberRoll {
        let cp_ns = mesh
            .enrollment
            .member_roll_namespace
            .as_deref()
            .unwrap_or(mesh.control_plane_namespace.as_str());
        if enroll_ossm_member_roll(client, cp_ns, namespace).await? {
            actions.push(format!(
                "added {namespace} to ServiceMeshMemberRoll in {cp_ns}"
            ));
        }
    }
    patch_namespace_enrollment_labels(client, namespace, mesh).await?;
    actions.push(format!(
        "set enrollment labels for mesh {} ({})",
        mesh.discovery_label, mesh.enrollment.revision
    ));
    Ok(actions)
}

async fn enroll_ossm_member_roll(
    client: &Client,
    control_plane_namespace: &str,
    namespace: &str,
) -> anyhow::Result<bool> {
    let members: BTreeSet<String> = collect_ossm_member_namespaces(client)
        .await
        .into_iter()
        .collect();
    if members.contains(namespace) {
        return Ok(false);
    }

    let ar = api_resource(
        "maistra.io",
        "v1",
        "ServiceMeshMemberRoll",
        "servicemeshmemberrolls",
    );
    let rolls = list_cr_in_namespace(client, &ar, control_plane_namespace).await?;
    let Some(roll) = rolls.into_iter().next() else {
        anyhow::bail!(
            "no ServiceMeshMemberRoll in {control_plane_namespace}; create one before enrollment"
        );
    };
    let name = roll
        .metadata
        .name
        .as_deref()
        .unwrap_or("default")
        .to_string();

    let mut member_list: Vec<String> = roll
        .data
        .get("spec")
        .and_then(|s| s.get("members"))
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    if member_list.iter().any(|m| m == namespace) {
        return Ok(false);
    }
    member_list.push(namespace.to_string());
    member_list.sort();
    member_list.dedup();

    let api = Api::<kube::api::DynamicObject>::namespaced_with(
        client.clone(),
        control_plane_namespace,
        &ar,
    );
    let patch = serde_json::json!({
        "spec": { "members": member_list }
    });
    api.patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(true)
}

async fn patch_namespace_enrollment_labels(
    client: &Client,
    namespace: &str,
    mesh: &MeshInstance,
) -> anyhow::Result<()> {
    use k8s_openapi::api::core::v1::Namespace;

    let labels = enrollment_labels_to_apply(mesh);
    let mut patch_labels = serde_json::Map::new();
    for (k, v) in labels {
        patch_labels.insert(k, serde_json::Value::String(v));
    }

    let api: Api<Namespace> = Api::all(client.clone());
    let patch = serde_json::json!({ "metadata": { "labels": patch_labels } });
    api.patch(namespace, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambientor_types::MeshEnrollmentMode;

    fn mesh_with_enrollment(enrollment: MeshEnrollment) -> MeshInstance {
        let discovery = enrollment
            .discovery_label_value
            .clone()
            .unwrap_or_else(|| "mesh-ambient".into());
        MeshInstance {
            revision: enrollment.revision.clone(),
            discovery_label: discovery,
            control_plane_namespace: "ambient-istio-system".into(),
            version: None,
            ambient: true,
            enrolled_namespace_count: 0,
            enrollment,
        }
    }

    #[test]
    fn enrolled_when_revision_and_discovery_match() {
        let enrollment = MeshEnrollment {
            mode: MeshEnrollmentMode::RevisionAndDiscovery,
            revision: "ambient-v1-28-6".into(),
            istio_revision: Some("ambient-v1-28-6".into()),
            revision_tag: None,
            discovery_label_key: Some("istio-discovery".into()),
            discovery_label_value: Some("mesh-ambient".into()),
            member_roll_namespace: None,
            from_istiod_config: true,
        };
        let mesh = mesh_with_enrollment(enrollment);
        let mut labels = BTreeMap::new();
        labels.insert("istio.io/rev".into(), "ambient-v1-28-6".into());
        labels.insert("istio-discovery".into(), "mesh-ambient".into());
        assert!(namespace_enrolled_on_mesh(&labels, &mesh));
    }

    #[test]
    fn conflicts_when_on_demo_mesh() {
        let enrollment = MeshEnrollment {
            mode: MeshEnrollmentMode::RevisionAndDiscovery,
            revision: "ambient-v1-28-6".into(),
            istio_revision: Some("ambient-v1-28-6".into()),
            revision_tag: None,
            discovery_label_key: Some("istio-discovery".into()),
            discovery_label_value: Some("mesh-ambient".into()),
            member_roll_namespace: None,
            from_istiod_config: false,
        };
        let mesh = mesh_with_enrollment(enrollment);
        let mut labels = BTreeMap::new();
        labels.insert("istio.io/rev".into(), "demo".into());
        labels.insert("istio-discovery".into(), "mesh-demo".into());
        let msg = namespace_conflicts_with_mesh(&labels, &mesh).unwrap();
        assert!(msg.contains("demo"));
    }

    #[test]
    fn parses_istiod_mesh_config() {
        let yaml = r#"
discoverySelectors:
- matchLabels:
    istio.io/rev: ambient-v1-28-6
    istio-discovery: mesh-ambient
"#;
        let parsed = parse_mesh_config_discovery(yaml).unwrap();
        assert_eq!(parsed.revision.as_deref(), Some("ambient-v1-28-6"));
        assert_eq!(
            parsed.discovery_label_value.as_deref(),
            Some("mesh-ambient")
        );
    }
}
