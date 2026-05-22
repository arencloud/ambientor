use ambientor_core::rules::{Rule, RuleContext, RuleId, finding};
use ambientor_types::{Finding, FindingCategory, FindingSeverity};

const MIXED_MODE_DOC: &str = "https://preliminary.istio.io/latest/docs/ambient/migrate/";

pub struct VsHttpRouteConflictRule;

impl Rule for VsHttpRouteConflictRule {
    fn id(&self) -> RuleId {
        "traffic.vs-httproute-conflict"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::TrafficCompatibility
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        if ctx.policies.virtual_services.is_empty() || ctx.policies.http_routes.is_empty() {
            return vec![];
        }
        vec![{
            let mut f = finding(
                self.id(),
                FindingSeverity::Blocker,
                self.category(),
                "VirtualService and HTTPRoute may conflict",
                "Mixing VirtualService and HTTPRoute for the same workload leads to undefined behavior during migration.",
            );
            f.doc_url = Some(MIXED_MODE_DOC.into());
            f.remediation =
                Some("Migrate each workload fully to HTTPRoute before enabling ambient".into());
            f
        }]
    }
}

pub struct L7WaypointRule;

impl Rule for L7WaypointRule {
    fn id(&self) -> RuleId {
        "traffic.l7-waypoint-required"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::TrafficCompatibility
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        ctx.policies
            .l7_authorization_policies
            .iter()
            .map(|name| {
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Warning,
                    self.category(),
                    "L7 policy requires waypoint",
                    format!(
                        "AuthorizationPolicy '{name}' uses L7 rules; deploy a waypoint proxy in affected namespaces."
                    ),
                );
                f.resource = Some(name.clone());
                f.doc_url = Some(MIXED_MODE_DOC.into());
                f.remediation = Some("Deploy waypoint before migrating namespaces with L7 AuthZ".into());
                f
            })
            .collect()
    }
}

pub struct MixedModeL7BypassRule;

impl Rule for MixedModeL7BypassRule {
    fn id(&self) -> RuleId {
        "traffic.mixed-mode-l7-bypass"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::TrafficCompatibility
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        let has_sidecar = ctx.namespaces.iter().any(|n| n.injection_enabled);
        let has_ambient = ctx.namespaces.iter().any(|n| n.ambient_enabled);
        if has_sidecar && has_ambient {
            vec![{
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Warning,
                    self.category(),
                    "Mixed-mode L7 policy gap",
                    "Traffic from sidecar workloads to ambient workloads with waypoints bypasses waypoint L7 enforcement until sources migrate.",
                );
                f.doc_url = Some(MIXED_MODE_DOC.into());
                f.remediation = Some(
                    "Migrate source namespaces to ambient before or together with destinations"
                        .into(),
                );
                f
            }]
        } else {
            vec![]
        }
    }
}

pub fn register_traffic_rules(registry: &mut ambientor_core::rules::RuleRegistry) {
    registry.register(Box::new(VsHttpRouteConflictRule));
    registry.register(Box::new(L7WaypointRule));
    registry.register(Box::new(MixedModeL7BypassRule));
}

pub fn traffic_registry() -> ambientor_core::rules::RuleRegistry {
    let mut registry = ambientor_core::rules::RuleRegistry::new();
    register_traffic_rules(&mut registry);
    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambientor_core::rules::{NamespaceContext, PolicyContext, RuleContext};

    #[test]
    fn mixed_mode_warning() {
        let ctx = RuleContext {
            namespaces: vec![
                NamespaceContext {
                    name: "a".into(),
                    injection_enabled: true,
                    ..Default::default()
                },
                NamespaceContext {
                    name: "b".into(),
                    ambient_enabled: true,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let findings = MixedModeL7BypassRule.evaluate(&ctx);
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn vs_httproute_blocker() {
        let ctx = RuleContext {
            policies: PolicyContext {
                virtual_services: vec!["vs".into()],
                http_routes: vec!["hr".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let findings = VsHttpRouteConflictRule.evaluate(&ctx);
        assert_eq!(findings[0].severity, FindingSeverity::Blocker);
    }
}
