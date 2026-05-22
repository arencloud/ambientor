use ambientor_core::rules::{Rule, RuleContext, RuleId, finding};
use ambientor_types::{Finding, FindingCategory, FindingSeverity};

const ISTIO_AMBIENT_MIGRATE: &str = "https://preliminary.istio.io/latest/docs/ambient/migrate/";

pub struct GatewayApiRule;

impl Rule for GatewayApiRule {
    fn id(&self) -> RuleId {
        "readiness.gateway-api"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::Readiness
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        if ctx.gateway_api_present {
            return vec![];
        }
        vec![{
            let mut f = finding(
                self.id(),
                FindingSeverity::Warning,
                self.category(),
                "Gateway API CRDs missing",
                "HTTPRoute and related Gateway API CRDs are required for ambient L7 routing.",
            );
            f.doc_url = Some(ISTIO_AMBIENT_MIGRATE.into());
            f.remediation = Some("Install Gateway API standard channel CRDs".into());
            f
        }]
    }
}

pub struct AmbientComponentsRule;

impl Rule for AmbientComponentsRule {
    fn id(&self) -> RuleId {
        "readiness.ambient-components"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::Readiness
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        if ctx.ambient_installed {
            return vec![];
        }
        vec![{
            let mut f = finding(
                self.id(),
                FindingSeverity::Warning,
                self.category(),
                "Ambient data plane not detected",
                "ztunnel DaemonSet was not found. Install ambient components before migrating workloads.",
            );
            f.doc_url = Some(ISTIO_AMBIENT_MIGRATE.into());
            f.remediation =
                Some("Upgrade Istio with ambient profile and verify ztunnel is Running".into());
            f
        }]
    }
}

pub struct PeerAuthDisableRule;

impl Rule for PeerAuthDisableRule {
    fn id(&self) -> RuleId {
        "readiness.peer-auth-disable"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::Readiness
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        ctx.policies
            .peer_auth_disable
            .iter()
            .map(|name| {
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Blocker,
                    self.category(),
                    "PeerAuthentication DISABLE cannot migrate",
                    format!(
                        "PeerAuthentication '{name}' uses mode DISABLE; ambient always enforces mTLS."
                    ),
                );
                f.resource = Some(name.clone());
                f.doc_url = Some(ISTIO_AMBIENT_MIGRATE.into());
                f.remediation =
                    Some("Remove or change PeerAuthentication to PERMISSIVE or STRICT".into());
                f
            })
            .collect()
    }
}

pub struct VmWorkloadRule;

impl Rule for VmWorkloadRule {
    fn id(&self) -> RuleId {
        "readiness.vm-workload"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::Readiness
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        ctx.namespaces
            .iter()
            .filter(|ns| ns.has_vm_workloads)
            .map(|ns| {
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Blocker,
                    self.category(),
                    "VM workloads cannot join ambient mesh",
                    format!(
                        "Namespace '{}' contains VM workloads which are not ambient-compatible.",
                        ns.name
                    ),
                );
                f.namespace = Some(ns.name.clone());
                f.doc_url = Some(ISTIO_AMBIENT_MIGRATE.into());
                f
            })
            .collect()
    }
}
