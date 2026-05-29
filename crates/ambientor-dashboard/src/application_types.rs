use std::collections::BTreeMap;

use ambientor_types::{AssessmentScores, Finding, FindingSummary};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationAssessmentRecord {
    pub namespace: String,
    pub mesh_revision: Option<String>,
    pub discovery_label: Option<String>,
    pub control_plane_namespace: Option<String>,
    pub hostnames: Vec<String>,
    pub namespace_labels: BTreeMap<String, String>,
    /// `ambient` | `sidecar` | `notEnrolled` — derived at assessment time.
    pub dataplane_mode: String,
    pub ingress_gateway_namespace: Option<String>,
    pub ingress_same_namespace: bool,
    pub workload_count: u32,
    pub readiness_pct: u8,
    pub risk_level: RiskLevel,
    pub blocker_count: u32,
    pub warning_count: u32,
    pub scores: AssessmentScores,
    pub summary: FindingSummary,
    pub findings: Vec<Finding>,
    pub suggestions: Vec<AssessmentSuggestion>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl RiskLevel {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        }
    }

    pub fn from_db_str(s: &str) -> Self {
        match s {
            "critical" => Self::Critical,
            "high" => Self::High,
            "medium" => Self::Medium,
            _ => Self::Low,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssessmentSuggestion {
    pub finding_id: String,
    pub severity: String,
    pub title: String,
    pub remediation: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClusterAssessmentRun {
    pub cluster_ref: String,
    pub applications: Vec<ApplicationAssessmentRecord>,
    pub cluster_scores: AssessmentScores,
    pub cluster_summary: FindingSummary,
    /// Findings that could not be mapped to a single namespace (shown cluster-wide in UI).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cluster_findings: Vec<Finding>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationListItem {
    pub namespace: String,
    pub cluster_ref: String,
    pub mesh_revision: Option<String>,
    pub discovery_label: Option<String>,
    pub control_plane_namespace: Option<String>,
    pub hostnames: Vec<String>,
    pub namespace_labels: BTreeMap<String, String>,
    pub dataplane_mode: String,
    pub ingress_gateway_namespace: Option<String>,
    pub ingress_same_namespace: bool,
    pub workload_count: u32,
    pub readiness_pct: u8,
    pub risk_level: RiskLevel,
    pub blocker_count: u32,
    pub warning_count: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationListPage {
    pub items: Vec<ApplicationListItem>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
    pub cluster_ref: String,
    pub run_id: Option<String>,
    pub last_assessed_at: Option<String>,
    #[serde(default)]
    pub cluster_summary: FindingSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cluster_findings: Vec<Finding>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplicationDetail {
    #[serde(flatten)]
    pub list: ApplicationListItem,
    pub scores: AssessmentScores,
    pub summary: FindingSummary,
    pub findings: Vec<Finding>,
    pub suggestions: Vec<AssessmentSuggestion>,
}
