use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicationMigrationStatus {
    Migrated,
    Processing,
    Blocker,
    Failed,
    Scanned,
    #[serde(rename = "notScanned")]
    NotScanned,
}

impl ApplicationMigrationStatus {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Migrated => "migrated",
            Self::Processing => "processing",
            Self::Blocker => "blocker",
            Self::Failed => "failed",
            Self::Scanned => "scanned",
            Self::NotScanned => "not_scanned",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        Some(match s {
            "migrated" => Self::Migrated,
            "processing" => Self::Processing,
            "blocker" => Self::Blocker,
            "failed" => Self::Failed,
            "scanned" => Self::Scanned,
            "not_scanned" => Self::NotScanned,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusCounts {
    pub migrated: usize,
    pub processing: usize,
    pub blocker: usize,
    pub failed: usize,
    pub scanned: usize,
    pub not_scanned: usize,
    pub total: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationRow {
    /// Display name from pod labels (falls back to namespace).
    pub application_name: String,
    pub namespace: String,
    pub status: ApplicationMigrationStatus,
    pub mesh_revision: String,
    pub discovery_label: String,
    /// `ambient` | `sidecar` | `notEnrolled`
    pub dataplane_mode: String,
    /// Deprecated alias; true only when `dataplane_mode == "ambient"`.
    pub ambient_dataplane: bool,
    pub blocker_count: usize,
    #[serde(default)]
    pub workload_count: u32,
    pub rollout_phase: Option<String>,
    pub assessment_ref: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshInstanceDashboard {
    pub revision: String,
    pub discovery_label: String,
    pub control_plane_namespace: String,
    pub version: Option<String>,
    pub ambient: bool,
    pub counts: StatusCounts,
    pub applications: Vec<ApplicationRow>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterDashboard {
    pub name: String,
    pub platform: String,
    pub mesh_flavor: String,
    pub istio_version: Option<String>,
    pub mesh_instance_count: usize,
    pub ambient_mesh_count: usize,
}

/// Estimated resource reduction after sidecar → ambient cutover (heuristic per workload).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationSavingsSummary {
    pub migrated_workloads: u32,
    pub estimated_sidecar_proxies_removed: u32,
    pub estimated_memory_mib_saved: u32,
    pub estimated_cpu_millicores_saved: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardResponse {
    pub cluster_ref: String,
    pub cluster: ClusterDashboard,
    pub summary: StatusCounts,
    pub mesh_instances: Vec<MeshInstanceDashboard>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migration_savings: Option<MigrationSavingsSummary>,
    pub last_updated: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetClusterDashboard {
    pub cluster_ref: String,
    pub cluster: ClusterDashboard,
    pub summary: StatusCounts,
    pub mesh_instances: Vec<MeshInstanceDashboard>,
    pub last_updated: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FleetDashboardResponse {
    pub summary: StatusCounts,
    pub clusters: Vec<FleetClusterDashboard>,
    pub last_updated: String,
}
