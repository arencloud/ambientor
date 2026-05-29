use std::collections::BTreeMap;

use ambientor_mesh::namespace_enrolled_on_mesh;
use ambientor_types::MeshInstance;

/// Effective namespace dataplane mode for dashboard and application catalog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataplaneMode {
    Ambient,
    /// Sidecar / legacy dataplane (enrolled on mesh, not ambient-labeled).
    Sidecar,
    /// No mesh enrollment labels detected.
    NotEnrolled,
}

impl DataplaneMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ambient => "ambient",
            Self::Sidecar => "sidecar",
            Self::NotEnrolled => "notEnrolled",
        }
    }

    pub fn is_ambient(self) -> bool {
        matches!(self, Self::Ambient)
    }
}

/// Derive dataplane mode from namespace labels and optional mesh instance metadata.
pub fn derive_dataplane_mode(
    labels: &BTreeMap<String, String>,
    mesh: Option<&MeshInstance>,
) -> DataplaneMode {
    if labels
        .get("istio.io/dataplane-mode")
        .is_some_and(|v| v == "ambient")
    {
        return DataplaneMode::Ambient;
    }

    let enrolled = mesh
        .map(|m| namespace_enrolled_on_mesh(labels, m))
        .unwrap_or_else(|| labels_indicate_mesh_membership(labels));

    if !enrolled {
        return DataplaneMode::NotEnrolled;
    }

    DataplaneMode::Sidecar
}

/// Same as [`derive_dataplane_mode`] when only stored assessment fields are available.
pub fn derive_dataplane_mode_from_stored(
    labels: &BTreeMap<String, String>,
    mesh_revision: Option<&str>,
    discovery_label: Option<&str>,
) -> DataplaneMode {
    if labels
        .get("istio.io/dataplane-mode")
        .is_some_and(|v| v == "ambient")
    {
        return DataplaneMode::Ambient;
    }
    if !labels_indicate_mesh_membership(labels) {
        return DataplaneMode::NotEnrolled;
    }
    if mesh_targets_ambient(mesh_revision, discovery_label) {
        return DataplaneMode::Sidecar;
    }
    DataplaneMode::Sidecar
}

pub fn mesh_targets_ambient(mesh_revision: Option<&str>, discovery_label: Option<&str>) -> bool {
    mesh_revision
        .is_some_and(|s| s.to_ascii_lowercase().contains("ambient"))
        || discovery_label
            .is_some_and(|s| s.to_ascii_lowercase().contains("ambient"))
}

pub fn labels_indicate_mesh_membership(labels: &BTreeMap<String, String>) -> bool {
    labels.contains_key("istio.io/rev")
        || labels.contains_key("istio-discovery")
        || labels
            .get("istio-injection")
            .is_some_and(|v| v == "enabled" || v == "true")
}

/// Namespace is scoped to an ambient Istio control plane (operational assessment path).
pub fn is_ambient_mesh_scope(dataplane: DataplaneMode, mesh: Option<&MeshInstance>) -> bool {
    dataplane == DataplaneMode::Ambient || mesh.is_some_and(|m| m.ambient)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambientor_types::{MeshEnrollment, MeshEnrollmentMode, MeshInstance};

    fn ambient_mesh() -> MeshInstance {
        MeshInstance {
            revision: "ambient-v1".into(),
            discovery_label: "mesh-ambient".into(),
            control_plane_namespace: "ambient-istio-system".into(),
            version: None,
            ambient: true,
            enrolled_namespace_count: 1,
            enrollment: MeshEnrollment {
                mode: MeshEnrollmentMode::DiscoveryLabel,
                revision: "ambient-v1".into(),
                discovery_label_key: Some("istio-discovery".into()),
                discovery_label_value: Some("mesh-ambient".into()),
                member_roll_namespace: None,
                from_istiod_config: false,
            },
        }
    }

    #[test]
    fn ambient_from_dataplane_label() {
        let mut labels = BTreeMap::new();
        labels.insert("istio.io/dataplane-mode".into(), "ambient".into());
        assert_eq!(
            derive_dataplane_mode(&labels, None),
            DataplaneMode::Ambient
        );
    }

    #[test]
    fn sidecar_when_enrolled_on_ambient_cp_without_label() {
        let mesh = ambient_mesh();
        let mut labels = BTreeMap::new();
        labels.insert("istio-discovery".into(), "mesh-ambient".into());
        assert_eq!(
            derive_dataplane_mode(&labels, Some(&mesh)),
            DataplaneMode::Sidecar
        );
        assert!(is_ambient_mesh_scope(DataplaneMode::Sidecar, Some(&mesh)));
    }

    #[test]
    fn not_enrolled_without_labels() {
        assert_eq!(derive_dataplane_mode(&BTreeMap::new(), None), DataplaneMode::NotEnrolled);
    }
}
