use ambientor_core::rules::{Rule, RuleContext, RuleId, finding};
use ambientor_types::{Finding, FindingCategory, FindingSeverity};

/// Detects workloads likely depending on sidecar localhost admin interface.
pub struct LocalhostProxyRule;

impl Rule for LocalhostProxyRule {
    fn id(&self) -> RuleId {
        "sidecar.localhost-proxy"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::SidecarDependency
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        let mut findings = Vec::new();
        for ns in &ctx.namespaces {
            if ns.injection_enabled && ns.workload_count > 0 {
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Info,
                    self.category(),
                    "Review sidecar localhost dependencies",
                    format!(
                        "Namespace '{}' has injected workloads; scan container specs for 127.0.0.1:15000/15001 usage.",
                        ns.name
                    ),
                );
                f.namespace = Some(ns.name.clone());
                f.remediation = Some(
                    "Replace localhost Envoy admin calls with mesh-native observability".into(),
                );
                findings.push(f);
            }
        }
        findings
    }
}

pub struct HoldUntilProxyRule;

impl Rule for HoldUntilProxyRule {
    fn id(&self) -> RuleId {
        "sidecar.hold-until-proxy"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::SidecarDependency
    }

    fn evaluate(&self, _ctx: &RuleContext) -> Vec<Finding> {
        vec![]
    }
}

#[cfg(test)]
mod tests {
    use ambientor_core::rules::{Rule, RuleContext};

    use crate::readiness::GatewayApiRule;

    #[test]
    fn gateway_api_rule_fires_when_missing() {
        let ctx = RuleContext {
            gateway_api_present: false,
            ..Default::default()
        };
        let findings = GatewayApiRule.evaluate(&ctx);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].id, "readiness.gateway-api");
    }
}
