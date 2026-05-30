use ambientor_core::rules::{Rule, RuleContext, RuleId, finding};
use ambientor_core::migrate_doc::MIGRATE_DOC;
use ambientor_types::{Finding, FindingCategory, FindingSeverity};

fn parse_resource_namespace(resource: &str) -> Option<String> {
    let (ns, name) = resource.split_once('/')?;
    if ns.is_empty() || name.is_empty() {
        return None;
    }
    Some(ns.to_string())
}

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
                FindingSeverity::Warning,
                self.category(),
                "VirtualService and HTTPRoute must not coexist per workload",
                "Istio documents this under known limitations (not a hard unsupported feature): \
                 mixing VirtualService and HTTPRoute for the same workload causes undefined routing \
                 behavior in ambient mode. The cluster has both resource types present.",
            );
            f.doc_url = Some(MIGRATE_DOC.into());
            f.remediation = Some(
                "1. Inventory each application namespace: list VirtualServices and HTTPRoutes.\n\
                 2. For each workload, choose one API — migrate L7 rules to Gateway API HTTPRoute \
                 (recommended) or complete cutover on VirtualService only (alpha in ambient).\n\
                 3. Remove or narrow the conflicting resource so only one API governs routing.\n\
                 4. Re-run assessment before labeling the namespace `istio.io/dataplane-mode=ambient`."
                    .into(),
            );
            f.evidence = Some(format!(
                "virtualServices: {}\nhttpRoutes: {}",
                ctx.policies.virtual_services.len(),
                ctx.policies.http_routes.len()
            ));
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
                let ns = parse_resource_namespace(name.as_str());
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Warning,
                    self.category(),
                    "L7 AuthorizationPolicy requires a waypoint proxy",
                    format!(
                        "AuthorizationPolicy `{name}` contains L7 match rules (HTTP paths, methods, \
                         headers). In ambient mode, ztunnel handles L4 only; L7 enforcement happens \
                         at a waypoint proxy attached via `Gateway`/`GatewayClass` and `targetRefs`."
                    ),
                );
                f.resource = Some(name.clone());
                f.namespace = ns.clone();
                f.doc_url = Some(MIGRATE_DOC.into());
                f.remediation = Some(format!(
                    "1. Confirm the namespace{} needs L7 policy (not L4-only).\n\
                     2. Deploy an Istio waypoint for the namespace (see \"Do you need waypoint proxies?\" in the migrate guide).\n\
                     3. Update the AuthorizationPolicy to use `targetRefs` selecting the waypoint.\n\
                     4. Verify traffic with `istioctl proxy-config` / waypoint status before migrating dataplane mode.",
                    ns.map(|n| format!(" `{n}`")).unwrap_or_default()
                ));
                f.evidence = Some(format!(
                    "resource: {name}\nspec: L7 rules detected on AuthorizationPolicy"
                ));
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
                    "Mixed sidecar and ambient dataplanes weaken L7 enforcement",
                    "During incremental migration, traffic from a sidecar-injected client to an \
                     ambient destination with a waypoint bypasses the waypoint. L7 AuthorizationPolicy \
                     on that waypoint is not enforced for that path until the source is also ambient.",
                );
                f.doc_url = Some(MIGRATE_DOC.into());
                f.remediation = Some(
                    "1. Identify caller namespaces still on sidecar injection and callees on ambient + waypoint.\n\
                     2. Plan migration waves so both ends of critical L7 paths share the same dataplane mode.\n\
                     3. Migrate caller namespaces to ambient (label + rollout) or defer waypoint-dependent policies until callers move.\n\
                     4. Validate policy with synthetic traffic after each namespace pair is aligned."
                        .into(),
                );
                f
            }]
        } else {
            vec![]
        }
    }
}

pub struct DestinationRuleSubsetsRule;

impl Rule for DestinationRuleSubsetsRule {
    fn id(&self) -> RuleId {
        "traffic.destination-rule-subsets"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::TrafficCompatibility
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        ctx.policies
            .destination_rules_with_subsets
            .iter()
            .map(|name| {
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Warning,
                    self.category(),
                    "DestinationRule subsets need HTTPRoute backendRefs planning",
                    format!(
                        "DestinationRule `{name}` defines `spec.subsets` used for version routing. \
                         In ambient mode, subset-based routing with HTTPRoute typically requires \
                         version-specific Kubernetes Services as `backendRefs` rather than subset labels alone."
                    ),
                );
                f.resource = Some(name.clone());
                f.namespace = parse_resource_namespace(name.as_str());
                f.doc_url = Some(MIGRATE_DOC.into());
                f.remediation = Some(
                    "1. Document each subset label and backing Service/version.\n\
                     2. Create per-version Services (or use Gateway API backendRefs) matching your traffic split.\n\
                     3. Migrate VirtualService subset routes to HTTPRoute rules before ambient cutover.\n\
                     4. Retain DestinationRule traffic policies (connection pool, outlier detection) — waypoints still apply them."
                        .into(),
                );
                f.evidence = Some(format!("resource: {name}\nspec.subsets: present"));
                f
            })
            .collect()
    }
}

pub fn register_traffic_rules(registry: &mut ambientor_core::rules::RuleRegistry) {
    registry.register(Box::new(VsHttpRouteConflictRule));
    registry.register(Box::new(L7WaypointRule));
    registry.register(Box::new(MixedModeL7BypassRule));
    registry.register(Box::new(DestinationRuleSubsetsRule));
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
        assert_eq!(findings[0].severity, FindingSeverity::Warning);
    }

    #[test]
    fn destination_rule_subsets_warning() {
        let ctx = RuleContext {
            policies: PolicyContext {
                destination_rules_with_subsets: vec!["default/reviews".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let findings = DestinationRuleSubsetsRule.evaluate(&ctx);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, FindingSeverity::Warning);
        assert!(findings[0].remediation.is_some());
    }

    #[test]
    fn vs_httproute_is_warning_not_blocker() {
        let ctx = RuleContext {
            policies: PolicyContext {
                virtual_services: vec!["bookinfo/reviews".into()],
                http_routes: vec!["bookinfo/route".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let findings = VsHttpRouteConflictRule.evaluate(&ctx);
        assert_eq!(findings[0].severity, FindingSeverity::Warning);
    }
}
