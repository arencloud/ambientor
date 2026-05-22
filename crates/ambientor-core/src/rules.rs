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
    pub policies: PolicyContext,
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
    }
}
