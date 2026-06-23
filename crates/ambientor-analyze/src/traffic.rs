use ambientor_core::migrate_doc::MIGRATE_DOC;
use ambientor_core::rules::{Rule, RuleContext, RuleId, finding};
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
        use std::collections::HashMap;

        let mut vs_by_ns: HashMap<String, Vec<&str>> = HashMap::new();
        let mut hr_by_ns: HashMap<String, Vec<&str>> = HashMap::new();

        for r in &ctx.policies.virtual_services {
            if let Some(ns) = parse_resource_namespace(r) {
                vs_by_ns.entry(ns).or_default().push(r.as_str());
            }
        }
        for r in &ctx.policies.http_routes {
            if let Some(ns) = parse_resource_namespace(r) {
                hr_by_ns.entry(ns).or_default().push(r.as_str());
            }
        }

        let mut findings = Vec::new();
        for ns in vs_by_ns.keys() {
            let Some(vs_list) = vs_by_ns.get(ns) else {
                continue;
            };
            let Some(hr_list) = hr_by_ns.get(ns) else {
                continue;
            };
            let mut f = finding(
                self.id(),
                FindingSeverity::Warning,
                self.category(),
                "VirtualService and HTTPRoute must not coexist in the same namespace",
                format!(
                    "Namespace `{ns}` defines both VirtualService and HTTPRoute resources. \
                     Istio documents this under known limitations (not a hard unsupported feature): \
                     mixing both APIs for routing in one namespace causes undefined behavior in ambient mode."
                ),
            );
            f.namespace = Some(ns.clone());
            f.doc_url = Some(MIGRATE_DOC.into());
            f.remediation = Some(format!(
                "1. In namespace `{ns}`, list VirtualServices and HTTPRoutes.\n\
                 2. For each workload, choose one API — migrate L7 rules to Gateway API HTTPRoute \
                 (recommended) or complete cutover on VirtualService only (alpha in ambient).\n\
                 3. Remove or narrow the conflicting resource so only one API governs routing.\n\
                 4. Re-run assessment before labeling `{ns}` with `istio.io/dataplane-mode=ambient`."
            ));
            f.evidence = Some(format!(
                "namespace: {ns}\nvirtualServices:\n{}\nhttpRoutes:\n{}",
                vs_list.join("\n"),
                hr_list.join("\n")
            ));
            findings.push(f);
        }
        findings
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

const AMBIENT_INGRESS_REMEDIATION: &str = "\
Option A — Shared ambient ingress (recommended when many apps use one gateway namespace):\n\
1. In the ingress namespace (e.g. bookinfo-gateway), create a Gateway API Gateway with \
   `gatewayClassName: istio` and label `istio.io/rev` matching your ambient control plane \
   (e.g. ambient-v1-28-6), or enroll that namespace on the ambient mesh.\n\
2. Update OpenShift Routes / LoadBalancer to target the new ambient ingress Service \
   (`<gateway-name>-istio`).\n\
3. Point each app HTTPRoute `parentRefs` at the ambient Gateway before or right after \
   migrating the app namespace to ambient.\n\
4. Run the public URL verification checklist (below).\n\
\n\
Option B — Per-namespace ingress (dedicated north–south for one app):\n\
1. After migrating the app namespace, create a Gateway on the ambient revision in that \
   namespace (or a small dedicated ingress namespace).\n\
2. Attach HTTPRoutes to that Gateway; update external Route/DNS to the new Service.\n\
3. Retire the old sidecar-revision parentRef when traffic is verified.\n\
\n\
Public URL verification (run before and after migration):\n\
1. `kubectl get httproute -n <app-ns> <name> -o jsonpath='{.status.parents}'` — must list \
   accepted parents (not empty).\n\
2. `kubectl get gateway -n <gw-ns> <name> -o jsonpath='{.status.conditions[?(@.type==\"Programmed\")].status}'` \
   — must be True.\n\
3. `curl -sI https://<public-host>/<path>` — expect the same HTTP status as before migration \
   (e.g. 200 or 302 to /productpage).\n\
4. OpenShift: `oc get route -n <gw-ns>` — backend Service must match the ambient ingress Service.";

pub struct AmbientIngressGatewayRule;

impl Rule for AmbientIngressGatewayRule {
    fn id(&self) -> RuleId {
        "traffic.ambient-ingress-gateway"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::TrafficCompatibility
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        use ambientor_mesh::ingress_collect::{
            gateway_for_route, has_programmed_ambient_ingress, route_uses_sidecar_ingress,
        };

        if !ctx.ambient_installed || ctx.policies.external_routes.is_empty() {
            return vec![];
        }

        let gateways = &ctx.policies.ingress_gateways;
        let ambient_ingress_ready = has_programmed_ambient_ingress(gateways);
        let mut findings = Vec::new();
        let mut seen_ns = std::collections::BTreeSet::new();

        for route in &ctx.policies.external_routes {
            let ns_ctx = ctx.namespaces.iter().find(|n| n.name == route.namespace);
            let Some(ns_ctx) = ns_ctx else {
                continue;
            };
            if ns_ctx.workload_count == 0 {
                continue;
            }
            let is_candidate = ns_ctx.injection_enabled && !ns_ctx.ambient_enabled;
            let is_ambient = ns_ctx.ambient_enabled;
            if !is_candidate && !is_ambient {
                continue;
            }

            let uses_sidecar_gw = route_uses_sidecar_ingress(route, gateways);
            let detached = route.parents_attached == Some(false);
            let needs_warning = if is_ambient {
                uses_sidecar_gw || detached || !ambient_ingress_ready
            } else {
                uses_sidecar_gw && !ambient_ingress_ready
            };
            if !needs_warning || !seen_ns.insert(route.namespace.clone()) {
                continue;
            }

            let gw = gateway_for_route(route, gateways);
            let gw_desc = gw
                .map(|g| format!("{}/{}", g.namespace, g.name))
                .unwrap_or_else(|| {
                    route
                        .parent_gateway_namespace
                        .as_ref()
                        .zip(route.parent_gateway_name.as_ref())
                        .map(|(a, b)| format!("{a}/{b}"))
                        .unwrap_or_else(|| "unknown gateway".into())
                });
            let hosts = if route.hostnames.is_empty() {
                "(see route spec)".to_string()
            } else {
                route.hostnames.join(", ")
            };

            let (title, message, severity) = if is_ambient {
                (
                    "External URL likely broken: ambient app still on sidecar ingress",
                    format!(
                        "Namespace `{ns}` is on ambient dataplane but north–south routes ({kind}/{name}) \
                         still target ingress gateway `{gw_desc}` on a sidecar/demo revision, or HTTPRoute \
                         parents are detached. Public URLs ({hosts}) will not work until an ambient \
                         ingress Gateway exists and routes attach to it.",
                        ns = route.namespace,
                        kind = route.kind,
                        name = route.name,
                        gw_desc = gw_desc,
                        hosts = hosts,
                    ),
                    FindingSeverity::Warning,
                )
            } else {
                (
                    "Plan ambient ingress before migration: public URL will break",
                    format!(
                        "Namespace `{ns}` exposes `{hosts}` via {kind}/{name} attached to sidecar-revision \
                         ingress `{gw_desc}`. Migrating workloads to ambient without an ambient ingress \
                         Gateway will detach HTTPRoutes and break external access.",
                        ns = route.namespace,
                        hosts = hosts,
                        kind = route.kind,
                        name = route.name,
                        gw_desc = gw_desc,
                    ),
                    FindingSeverity::Warning,
                )
            };

            let mut f = finding(self.id(), severity, self.category(), title, message);
            f.namespace = Some(route.namespace.clone());
            f.doc_url = Some(MIGRATE_DOC.into());
            f.remediation = Some(AMBIENT_INGRESS_REMEDIATION.into());
            f.evidence = Some(format!(
                "namespace: {}\nroute: {}/{}\nhostnames: {}\nparentGateway: {}\nparentsAttached: {:?}\nambientIngressProgrammed: {}",
                route.namespace,
                route.kind,
                route.name,
                hosts,
                gw_desc,
                route.parents_attached,
                ambient_ingress_ready,
            ));
            findings.push(f);
        }
        findings
    }
}

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
    registry.register(Box::new(AmbientIngressGatewayRule));
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
    fn ambient_ingress_warning_before_migration() {
        use ambientor_core::rules::{ExternalRouteInfo, IngressGatewayInfo, PolicyContext};

        let ctx = RuleContext {
            ambient_installed: true,
            namespaces: vec![NamespaceContext {
                name: "bookinfo-demo1".into(),
                injection_enabled: true,
                ambient_enabled: false,
                workload_count: 4,
                ..Default::default()
            }],
            policies: PolicyContext {
                ingress_gateways: vec![IngressGatewayInfo {
                    namespace: "bookinfo-gateway".into(),
                    name: "demo-gw".into(),
                    istio_revision: Some("demo".into()),
                    discovery_label: Some("mesh-demo".into()),
                    programmed: true,
                    gateway_class: Some("istio".into()),
                }],
                external_routes: vec![ExternalRouteInfo {
                    namespace: "bookinfo-demo1".into(),
                    name: "bookinfo".into(),
                    kind: "HTTPRoute".into(),
                    hostnames: vec!["demo1.example.com".into()],
                    parent_gateway_namespace: Some("bookinfo-gateway".into()),
                    parent_gateway_name: Some("demo-gw".into()),
                    parents_attached: Some(true),
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let findings = AmbientIngressGatewayRule.evaluate(&ctx);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].id, "traffic.ambient-ingress-gateway");
        assert!(
            findings[0]
                .remediation
                .as_ref()
                .is_some_and(|r| r.contains("Option A"))
        );
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
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, FindingSeverity::Warning);
        assert_eq!(findings[0].namespace.as_deref(), Some("bookinfo"));
    }

    #[test]
    fn vs_httproute_no_conflict_when_types_in_different_namespaces() {
        let ctx = RuleContext {
            policies: PolicyContext {
                virtual_services: vec!["bookinfo-direct-4/reviews".into()],
                http_routes: vec![
                    "bookinfo-direct-1/route".into(),
                    "bookinfo-direct-2/route".into(),
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let findings = VsHttpRouteConflictRule.evaluate(&ctx);
        assert!(findings.is_empty());
    }
}
