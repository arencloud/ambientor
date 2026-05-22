use ambientor_core::rules::{Rule, RuleContext, RuleId, finding};
use ambientor_types::{Finding, FindingCategory, FindingSeverity};

const SIDECAR_MIGRATE_DOC: &str = "https://preliminary.istio.io/latest/docs/ambient/migrate/";

/// Detects workloads calling the sidecar Envoy admin interface on localhost.
pub struct LocalhostProxyRule;

impl Rule for LocalhostProxyRule {
    fn id(&self) -> RuleId {
        "sidecar.localhost-proxy"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::SidecarDependency
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        ctx.workloads
            .iter()
            .filter(|w| w.uses_localhost_proxy)
            .map(|w| {
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Warning,
                    self.category(),
                    "Workload uses sidecar localhost admin port",
                    format!(
                        "Pod '{}/{}' references 127.0.0.1:15000/15001 or localhost:15000/15001 in container configuration.",
                        w.namespace, w.name
                    ),
                );
                f.namespace = Some(w.namespace.clone());
                f.resource = Some(format!("Pod/{}/{}", w.namespace, w.name));
                f.doc_url = Some(SIDECAR_MIGRATE_DOC.into());
                f.remediation = Some(
                    "Replace localhost Envoy admin calls with mesh-native observability (e.g. Prometheus, ztunnel metrics)"
                        .into(),
                );
                f.evidence = Some(format!(
                    "pod: {}/{}\n{}",
                    w.namespace,
                    w.name,
                    w.localhost_proxy_hits.join("\n")
                ));
                f
            })
            .collect()
    }
}

/// Detects holdApplicationUntilProxyStarts — incompatible with ambient startup ordering.
pub struct HoldUntilProxyRule;

impl Rule for HoldUntilProxyRule {
    fn id(&self) -> RuleId {
        "sidecar.hold-until-proxy"
    }

    fn category(&self) -> FindingCategory {
        FindingCategory::SidecarDependency
    }

    fn evaluate(&self, ctx: &RuleContext) -> Vec<Finding> {
        ctx.workloads
            .iter()
            .filter(|w| w.hold_until_proxy)
            .map(|w| {
                let mut f = finding(
                    self.id(),
                    FindingSeverity::Blocker,
                    self.category(),
                    "holdApplicationUntilProxyStarts is configured",
                    format!(
                        "Pod '{}/{}' uses proxy.istio.io/config holdApplicationUntilProxyStarts; ambient workloads do not use the sidecar startup gate.",
                        w.namespace, w.name
                    ),
                );
                f.namespace = Some(w.namespace.clone());
                f.resource = Some(format!("Pod/{}/{}", w.namespace, w.name));
                f.doc_url = Some(SIDECAR_MIGRATE_DOC.into());
                f.remediation = Some(
                    "Remove holdApplicationUntilProxyStarts and use readiness probes or init ordering native to your platform"
                        .into(),
                );
                f.evidence = Some(format!(
                    "pod: {}/{}\nannotation: proxy.istio.io/config contains holdApplicationUntilProxyStarts",
                    w.namespace, w.name
                ));
                f
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use ambientor_core::rules::{Rule, RuleContext, WorkloadContext};
    use ambientor_types::FindingSeverity;

    use super::*;

    #[test]
    fn localhost_proxy_rule_fires_per_workload() {
        let ctx = RuleContext {
            workloads: vec![WorkloadContext {
                namespace: "bookinfo".into(),
                name: "reviews-abc".into(),
                uses_localhost_proxy: true,
                localhost_proxy_hits: vec!["app: env METRICS=http://127.0.0.1:15000/stats".into()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let findings = LocalhostProxyRule.evaluate(&ctx);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, FindingSeverity::Warning);
        assert!(
            findings[0]
                .evidence
                .as_ref()
                .is_some_and(|e| e.contains("15000"))
        );
    }

    #[test]
    fn hold_until_proxy_blocker() {
        let ctx = RuleContext {
            workloads: vec![WorkloadContext {
                namespace: "ns".into(),
                name: "app".into(),
                hold_until_proxy: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let findings = HoldUntilProxyRule.evaluate(&ctx);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, FindingSeverity::Blocker);
    }
}
