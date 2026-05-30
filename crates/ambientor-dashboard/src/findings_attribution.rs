//! Map cluster-scoped rule findings onto application namespaces for the catalog.

use std::collections::{BTreeSet, HashMap};

use ambientor_core::rules::RuleContext;
use ambientor_types::Finding;

/// Namespace from `Finding.namespace` or `resource` (`namespace/name`).
pub fn finding_namespace(f: &Finding) -> Option<String> {
    if let Some(ns) = &f.namespace {
        if !ns.is_empty() {
            return Some(ns.clone());
        }
    }
    f.resource
        .as_ref()
        .and_then(|r| parse_resource_namespace(r))
}

pub fn parse_resource_namespace(resource: &str) -> Option<String> {
    let (ns, name) = resource.split_once('/')?;
    if ns.is_empty() || name.is_empty() {
        return None;
    }
    Some(ns.to_string())
}

/// Partition findings per namespace; returns unattributed cluster findings separately.
pub fn partition_findings_by_namespace(
    all_findings: &[Finding],
    ctx: &RuleContext,
) -> (HashMap<String, Vec<Finding>>, Vec<Finding>) {
    let mut by_ns: HashMap<String, Vec<Finding>> = HashMap::new();
    let mut cluster_only = Vec::new();

    for f in all_findings {
        if let Some(ns) = finding_namespace(f) {
            by_ns.entry(ns).or_default().push(f.clone());
            continue;
        }

        let targets = cluster_finding_target_namespaces(f, ctx);
        if targets.is_empty() {
            cluster_only.push(f.clone());
            continue;
        }
        for ns in targets {
            by_ns.entry(ns).or_default().push(f.clone());
        }
    }

    (by_ns, cluster_only)
}

/// Namespaces affected by a finding with no explicit `namespace` field.
fn cluster_finding_target_namespaces(f: &Finding, ctx: &RuleContext) -> Vec<String> {
    match f.id.as_str() {
        "traffic.vs-httproute-conflict" => namespaces_from_policy_refs(
            ctx.policies.virtual_services.iter(),
            ctx.policies.http_routes.iter(),
        ),
        "traffic.mixed-mode-l7-bypass" => ctx
            .namespaces
            .iter()
            .filter(|n| n.injection_enabled || n.ambient_enabled)
            .map(|n| n.name.clone())
            .collect(),
        "readiness.gateway-api" | "readiness.ambient-components" | "readiness.istio-version" => {
            ctx.namespaces
                .iter()
                .filter(|n| n.workload_count > 0)
                .map(|n| n.name.clone())
                .collect()
        }
        _ => Vec::new(),
    }
}

fn namespaces_from_policy_refs<'a>(
    virtual_services: impl Iterator<Item = &'a String>,
    http_routes: impl Iterator<Item = &'a String>,
) -> Vec<String> {
    let mut set = BTreeSet::new();
    for r in virtual_services.chain(http_routes) {
        if let Some(ns) = parse_resource_namespace(r) {
            set.insert(ns);
        }
    }
    set.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambientor_core::rules::PolicyContext;
    use ambientor_types::{FindingCategory, FindingSeverity};

    #[test]
    fn parses_namespace_from_resource_ref() {
        let f = Finding {
            id: "traffic.destination-rule-subsets".into(),
            severity: FindingSeverity::Warning,
            category: FindingCategory::TrafficCompatibility,
            title: String::new(),
            message: String::new(),
            namespace: None,
            resource: Some("demo-vm/dr-egressgateway".into()),
            remediation: None,
            doc_url: None,
            evidence: None,
        };
        assert_eq!(
            finding_namespace(&f).as_deref(),
            Some("demo-vm")
        );
    }

    #[test]
    fn fans_out_vs_httproute_blocker_to_policy_namespaces() {
        let f = Finding {
            id: "traffic.vs-httproute-conflict".into(),
            severity: FindingSeverity::Warning,
            category: FindingCategory::TrafficCompatibility,
            title: String::new(),
            message: String::new(),
            namespace: None,
            resource: None,
            remediation: None,
            doc_url: None,
            evidence: None,
        };
        let ctx = RuleContext {
            policies: PolicyContext {
                virtual_services: vec!["bookinfo/reviews".into()],
                http_routes: vec!["mesh-sidecar-1/app-route".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        let (by_ns, cluster) = partition_findings_by_namespace(&[f], &ctx);
        assert!(cluster.is_empty());
        assert!(by_ns.contains_key("bookinfo"));
        assert!(by_ns.contains_key("mesh-sidecar-1"));
    }
}
