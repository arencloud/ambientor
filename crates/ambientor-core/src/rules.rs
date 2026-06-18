use ambientor_types::{Finding, FindingCategory, FindingSeverity};
use serde::{Deserialize, Serialize};

/// Unique rule identifier in the registry.
pub type RuleId = &'static str;

pub trait Rule: Send + Sync {
    fn id(&self) -> RuleId;
    fn category(&self) -> FindingCategory;
    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding>;
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleContext {
    pub mesh_version: Option<String>,
    pub mesh_flavor: Option<String>,
    pub ambient_installed: bool,
    pub gateway_api_present: bool,
    pub namespaces: Vec<NamespaceContext>,
    pub workloads: Vec<WorkloadContext>,
    pub policies: PolicyContext,
    pub platform: PlatformContext,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformContext {
    pub spire_detected: bool,
    #[serde(default)]
    pub spire_hits: Vec<String>,
    #[serde(default)]
    pub ossm_member_namespaces: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NamespaceContext {
    pub name: String,
    pub injection_enabled: bool,
    pub ambient_enabled: bool,
    pub workload_count: u32,
    pub has_vm_workloads: bool,
}

/// Per-pod signals used by sidecar dependency rules.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkloadContext {
    pub namespace: String,
    pub name: String,
    pub has_istio_sidecar: bool,
    pub uses_localhost_proxy: bool,
    #[serde(default)]
    pub localhost_proxy_hits: Vec<String>,
    pub hold_until_proxy: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PolicyContext {
    #[serde(default)]
    pub peer_auth_disable: Vec<String>,
    #[serde(default)]
    pub envoy_filters: Vec<String>,
    #[serde(default)]
    pub virtual_services: Vec<String>,
    #[serde(default)]
    pub http_routes: Vec<String>,
    #[serde(default)]
    pub l7_authorization_policies: Vec<String>,
    #[serde(default)]
    pub destination_rules: Vec<String>,
    #[serde(default)]
    pub destination_rules_with_subsets: Vec<String>,
    #[serde(default)]
    pub envoy_filters_waypoint: Vec<String>,
    #[serde(default)]
    pub ingress_gateways: Vec<IngressGatewayInfo>,
    #[serde(default)]
    pub external_routes: Vec<ExternalRouteInfo>,
}

/// Gateway API `Gateway` used for north–south ingress (shared or dedicated).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngressGatewayInfo {
    pub namespace: String,
    pub name: String,
    pub istio_revision: Option<String>,
    pub discovery_label: Option<String>,
    pub programmed: bool,
    pub gateway_class: Option<String>,
}

/// HTTPRoute or VirtualService that exposes an app externally via a gateway.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExternalRouteInfo {
    pub namespace: String,
    pub name: String,
    pub kind: String,
    pub hostnames: Vec<String>,
    pub parent_gateway_namespace: Option<String>,
    pub parent_gateway_name: Option<String>,
    /// When known from HTTPRoute status: route accepted by parent gateway.
    pub parents_attached: Option<bool>,
}

pub struct RuleRegistry {
    rules: Vec<Box<dyn Rule>>,
}

impl RuleRegistry {
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    pub fn register(&mut self, rule: Box<dyn Rule>) {
        self.rules.push(rule);
    }

    pub fn evaluate_all(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        for rule in &self.rules {
            findings.extend(rule.evaluate(ctx));
        }
        findings
    }
}

impl Default for RuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn finding(
    id: &str,
    severity: FindingSeverity,
    category: FindingCategory,
    title: impl Into<String>,
    message: impl Into<String>,
) -> Finding {
    Finding {
        id: id.to_string(),
        severity,
        category,
        title: title.into(),
        message: message.into(),
        namespace: None,
        resource: None,
        remediation: None,
        doc_url: None,
        evidence: None,
    }
}
