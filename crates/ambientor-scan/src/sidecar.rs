use ambientor_core::migrate_doc::MIGRATE_DOC;
use ambientor_core::rules::{Rule, RuleContext, RuleId, finding};
use ambientor_types::{Finding, FindingCategory, FindingSeverity};

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
                    "Workload references sidecar Envoy admin on localhost",
                    format!(
                        "Pod '{}/{}' uses 127.0.0.1:15000/15001 or localhost:15000/15001 in configuration. \
                         This pattern assumes a local sidecar admin port; ambient uses ztunnel and waypoint \
                         metrics instead.",
                        w.namespace, w.name
                    ),
                );
                f.namespace = Some(w.namespace.clone());
                f.resource = Some(format!("Pod/{}/{}", w.namespace, w.name));
                f.doc_url = Some(MIGRATE_DOC.into());
                f.remediation = Some(
                    "1. Find env vars, probes, or scripts referencing localhost:15000/15001.\n\
                     2. Switch observability to mesh-native metrics (ztunnel, waypoint, application Prometheus).\n\
                     3. Remove admin-port dependencies before removing sidecar injection.\n\
                     4. Re-run assessment on the namespace."
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
                    FindingSeverity::Warning,
                    self.category(),
                    "holdApplicationUntilProxyStarts requires sidecar startup ordering",
                    format!(
                        "Pod '{}/{}' sets `proxy.istio.io/config` with holdApplicationUntilProxyStarts. \
                         Ambient workloads do not use the sidecar startup gate; pods may hang or behave \
                         differently without the sidecar proxy.",
                        w.namespace, w.name
                    ),
                );
                f.namespace = Some(w.namespace.clone());
                f.resource = Some(format!("Pod/{}/{}", w.namespace, w.name));
                f.doc_url = Some(MIGRATE_DOC.into());
                f.remediation = Some(
                    "1. Remove holdApplicationUntilProxyStarts from the pod/deployment template.\n\
                     2. Use native Kubernetes readiness/liveness probes for startup ordering.\n\
                     3. If the app truly requires sidecar startup synchronization, keep the workload on sidecar mode until refactored.\n\
                     4. Re-run assessment after template changes."
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
    }

    #[test]
    fn hold_until_proxy_warning() {
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
        assert_eq!(findings[0].severity, FindingSeverity::Warning);
    }
}
