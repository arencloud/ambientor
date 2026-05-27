use chrono::{DateTime, Utc};
use kube::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::dto::{Finding, FindingSummary};

pub const GROUP: &str = "ambientor.io";
pub const VERSION: &str = "v1alpha1";

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

/// Resolved mesh control plane (istiod revision + discovery label).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MeshInstance {
    pub revision: String,
    pub discovery_label: String,
    pub control_plane_namespace: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub ambient: bool,
    pub enrolled_namespace_count: usize,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment_ref: Option<String>,
    #[serde(default = "default_ambient")]
    pub target_mesh_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_target: Option<MeshTarget>,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "PascalCase")]
pub enum RolloutStageType {
    InstallAmbientComponents,
    DeployWaypoint,
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
    #[serde(default = "default_true")]
    pub auto_rollback: bool,
    /// When omitted and exactly one ambient control plane exists, it is selected automatically.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mesh_target: Option<MeshTarget>,
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
