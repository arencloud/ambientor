use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dto::{Finding, FindingSummary};

pub const GROUP: &str = "ambientor.io";
pub const VERSION: &str = "v1alpha1";

/// Shared ambient north–south Gateway API `Gateway` to attach app HTTPRoutes during migration.
/// When omitted, Ambientor creates a per-namespace ingress Gateway in each app namespace.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AmbientIngressGateway {
    pub namespace: String,
    pub name: String,
}

/// Selects which Istio / OSSM control plane a rollout or plan targets.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeshTarget {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_plane_namespace: Option<String>,
}

/// How a namespace is associated with a particular istiod / OSSM control plane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MeshEnrollmentMode {
    /// Namespace `istio.io/rev` must match the control-plane revision.
    RevisionOnly,
    /// Namespace must carry a discovery selector label (key/value from istiod mesh config).
    DiscoveryLabel,
    /// Both revision and discovery label are required (common on OSSM / Sail).
    RevisionAndDiscovery,
    /// OSSM `ServiceMeshMemberRoll` membership plus revision/discovery labels when configured.
    OssmMemberRoll,
}

/// Enrollment contract for a mesh instance, derived from istiod mesh config and cluster observation.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeshEnrollment {
    pub mode: MeshEnrollmentMode,
    /// Value for namespace `istio.io/rev` (revision tag name when configured, else istiod revision).
    pub revision: String,
    /// Underlying istiod control-plane revision (deployment label).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub istio_revision: Option<String>,
    /// Istio `RevisionTag` / `IstioRevisionTag` name when one targets this istiod revision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision_tag: Option<String>,
    /// Label key from istiod `discoverySelectors` (e.g. `istio-discovery`); absent when revision-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_label_key: Option<String>,
    /// Value namespaces must carry for discovery enrollment; absent when revision-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discovery_label_value: Option<String>,
    /// Control-plane namespace hosting `ServiceMeshMemberRoll` when mode is `ossmMemberRoll`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub member_roll_namespace: Option<String>,
    /// True when enrollment was read from istiod ConfigMap mesh config (not inferred).
    #[serde(default)]
    pub from_istiod_config: bool,
}

/// Resolved mesh control plane (istiod revision + discovery label).
#[derive(Clone, Debug, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeshInstance {
    pub revision: String,
    /// Primary discovery label value for UI/selection (same as `enrollment.discovery_label_value` when set).
    pub discovery_label: String,
    pub control_plane_namespace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub ambient: bool,
    pub enrolled_namespace_count: usize,
    pub enrollment: MeshEnrollment,
}

/// Build enrollment from pre–`MeshEnrollment` rollout status (`revision` + `discoveryLabel` only).
pub fn legacy_enrollment_from_mesh_instance(mesh: &MeshInstance) -> MeshEnrollment {
    MeshEnrollment {
        mode: MeshEnrollmentMode::RevisionAndDiscovery,
        revision: mesh.revision.clone(),
        istio_revision: Some(mesh.revision.clone()),
        revision_tag: None,
        discovery_label_key: Some("istio-discovery".into()),
        discovery_label_value: Some(mesh.discovery_label.clone()),
        member_roll_namespace: None,
        from_istiod_config: false,
    }
}

impl<'de> Deserialize<'de> for MeshInstance {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Raw {
            revision: String,
            discovery_label: String,
            control_plane_namespace: String,
            #[serde(default)]
            version: Option<String>,
            ambient: bool,
            #[serde(default)]
            enrolled_namespace_count: usize,
            #[serde(default)]
            enrollment: Option<MeshEnrollment>,
        }
        let raw = Raw::deserialize(deserializer)?;
        let enrollment = raw
            .enrollment
            .unwrap_or_else(|| legacy_enrollment_from_mesh_instance(&MeshInstance {
                revision: raw.revision.clone(),
                discovery_label: raw.discovery_label.clone(),
                control_plane_namespace: raw.control_plane_namespace.clone(),
                version: raw.version.clone(),
                ambient: raw.ambient,
                enrolled_namespace_count: raw.enrolled_namespace_count,
                enrollment: MeshEnrollment {
                    mode: MeshEnrollmentMode::RevisionOnly,
                    revision: String::new(),
                    istio_revision: None,
                    revision_tag: None,
                    discovery_label_key: None,
                    discovery_label_value: None,
                    member_roll_namespace: None,
                    from_istiod_config: false,
                },
            }));
        Ok(MeshInstance {
            revision: raw.revision,
            discovery_label: raw.discovery_label,
            control_plane_namespace: raw.control_plane_namespace,
            version: raw.version,
            ambient: raw.ambient,
            enrolled_namespace_count: raw.enrolled_namespace_count,
            enrollment,
        })
    }
}

#[cfg(test)]
mod mesh_instance_tests {
    use super::*;

    #[test]
    fn deserializes_rollout_status_without_enrollment_field() {
        let json = r#"{
            "revision": "ambient-v1-28-6",
            "discoveryLabel": "mesh-ambient",
            "controlPlaneNamespace": "ambient-istio-system",
            "ambient": true,
            "enrolledNamespaceCount": 2
        }"#;
        let mesh: MeshInstance = serde_json::from_str(json).expect("deserialize legacy status");
        assert_eq!(mesh.enrollment.revision, "ambient-v1-28-6");
        assert_eq!(
            mesh.enrollment.discovery_label_value.as_deref(),
            Some("mesh-ambient")
        );
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum MeshFlavor {
    UpstreamIstio,
    OSSM3,
    GenericKubernetes,
    #[default]
    Unknown,
}

#[derive(CustomResource, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "ambientor.io",
    version = "v1alpha1",
    kind = "Cluster",
    namespaced = false,
    status = "ClusterStatus",
    printcolumn = r#"{"name":"Mesh", "type":"string", "jsonPath":".spec.meshFlavor"}"#,
    printcolumn = r#"{"name":"Version", "type":"string", "jsonPath":".status.meshVersion"}"#
)]
#[serde(rename_all = "camelCase")]
pub struct ClusterSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub mesh_flavor: MeshFlavor,
    #[serde(default = "default_true")]
    pub in_cluster: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClusterStatus {
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub istio_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,
}

#[derive(CustomResource, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "ambientor.io",
    version = "v1alpha1",
    kind = "ClusterConnection",
    namespaced = true,
    status = "ClusterConnectionStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct ClusterConnectionSpec {
    pub display_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_server: Option<String>,
    pub credentials_secret_ref: SecretRef,
    #[serde(default)]
    pub hub: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretRef {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ClusterConnectionStatus {
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,
}

#[derive(CustomResource, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "ambientor.io",
    version = "v1alpha1",
    kind = "MeshInventory",
    namespaced = true,
    status = "MeshInventoryStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct MeshInventorySpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace_selector: Option<LabelSelector>,
    #[serde(default)]
    pub trigger_scan: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LabelSelector {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub match_labels: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MeshInventoryStatus {
    pub phase: String,
    #[serde(default)]
    pub generation: i64,
    /// Last processed `metadata.generation` when a scan was triggered.
    #[serde(default)]
    pub observed_generation: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_scan_time: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment_ref: Option<String>,
}

#[derive(CustomResource, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "ambientor.io",
    version = "v1alpha1",
    kind = "AmbientAssessment",
    namespaced = true,
    status = "AmbientAssessmentStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct AmbientAssessmentSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inventory_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_ref: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AmbientAssessmentStatus {
    pub phase: String,
    #[serde(default)]
    pub readiness_score: u8,
    #[serde(default)]
    pub sidecar_dependency_score: u8,
    #[serde(default)]
    pub traffic_compatibility_score: u8,
    #[serde(default)]
    pub overall_score: u8,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<FindingSummary>,
}

#[derive(CustomResource, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "ambientor.io",
    version = "v1alpha1",
    kind = "MigrationPlan",
    namespaced = true,
    status = "MigrationPlanStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPlanSpec {
    /// Optional link to an assessment (policy hints only when `selectedNamespaces` is set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment_ref: Option<String>,
    /// User-selected namespaces to migrate (preferred over assessment-wide expansion).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub selected_namespaces: Vec<String>,
    /// Fleet / multicluster identity for this plan (defaults to operator `CLUSTER_REF`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_ref: Option<String>,
    /// Human-readable label in UI and exports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default = "default_ambient")]
    pub target_mesh_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_target: Option<MeshTarget>,
    /// Optional shared ambient ingress Gateway; when unset, each wave namespace gets its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient_ingress_gateway: Option<AmbientIngressGateway>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waves: Vec<MigrationWave>,
}

fn default_ambient() -> String {
    "ambient".to_string()
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationWave {
    pub name: String,
    pub namespaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prerequisites: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policy_tasks: Vec<PolicyTask>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PolicyTask {
    pub kind: String,
    pub name: String,
    pub namespace: String,
    pub action: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MigrationPlanStatus {
    pub phase: String,
    #[serde(default)]
    pub approved: bool,
    #[serde(default)]
    pub wave_count: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_count: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_ref: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum RolloutStageType {
    InstallAmbientComponents,
    /// Enroll namespaces on the rollout mesh target (labels, OSSM MemberRoll, etc.).
    EnrollNamespace,
    DeployWaypoint,
    /// Create or select ambient ingress Gateway and repoint HTTPRoutes (and OpenShift Routes).
    MigrateIngress,
    LabelNamespace,
    TranslatePolicy,
    RollingRestart,
    RemoveInjection,
    VerifyTraffic,
    DryRun,
}

#[derive(CustomResource, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "ambientor.io",
    version = "v1alpha1",
    kind = "Rollout",
    namespaced = true,
    status = "RolloutStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct RolloutSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_ref: Option<String>,
    /// Fleet cluster identity (from MigrationPlan when created via API).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_ref: Option<String>,
    #[serde(default = "default_true")]
    pub auto_rollback: bool,
    /// When omitted and exactly one ambient control plane exists, it is selected automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_target: Option<MeshTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient_ingress_gateway: Option<AmbientIngressGateway>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stages: Vec<RolloutStage>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RolloutStage {
    pub name: String,
    pub r#type: RolloutStageType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub namespaces: Vec<String>,
    #[serde(default = "default_true")]
    pub requires_approval: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RolloutStatus {
    pub phase: String,
    #[serde(default)]
    pub current_stage: i32,
    #[serde(default)]
    pub approved_stage: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_mesh_target: Option<MeshInstance>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<Condition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stage_results: Vec<StageResult>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StageResult {
    pub name: String,
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(CustomResource, Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[kube(
    group = "ambientor.io",
    version = "v1alpha1",
    kind = "PolicyTranslation",
    namespaced = true,
    status = "PolicyTranslationStatus"
)]
#[serde(rename_all = "camelCase")]
pub struct PolicyTranslationSpec {
    pub source_kind: String,
    pub source_name: String,
    pub target_kind: String,
    pub namespace: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PolicyTranslationStatus {
    pub phase: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_manifest: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    pub r#type: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}
