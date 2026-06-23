//! Per-application deep investigation for namespaces on ambient Istio control planes.

use std::collections::BTreeSet;

use ambientor_core::rules::RuleContext;
use ambientor_types::{Finding, FindingCategory, FindingSeverity, MeshInstance};

use crate::application_types::AssessmentSuggestion;
use crate::dataplane::{DataplaneMode, is_ambient_mesh_scope};

const AMBIENT_MIGRATE_DOC: &str = "https://preliminary.istio.io/latest/docs/ambient/migrate/";

/// Enrich assessment with operational deep-investigation findings and suggestions.
#[allow(clippy::too_many_arguments)]
pub fn enrich_ambient_application(
    ns_name: &str,
    dataplane: DataplaneMode,
    mesh: Option<&MeshInstance>,
    ctx: &RuleContext,
    findings: &mut Vec<Finding>,
    suggestions: &mut Vec<AssessmentSuggestion>,
    hostnames_empty: bool,
    workload_count: u32,
) {
    if !is_ambient_mesh_scope(dataplane, mesh) {
        return;
    }

    findings.retain(|f| f.id != "enrollment.missing-ambient-dataplane");

    if hostnames_empty {
        append_hostname_inventory(ns_name, mesh, workload_count, findings, suggestions);
    }

    append_workload_investigation(ns_name, dataplane, ctx, findings, suggestions);
    append_policy_investigation(ns_name, ctx, findings, suggestions);
    append_namespace_rule_synthesis(ns_name, findings, suggestions);

    if workload_count > 0 && !has_open_issues(findings) {
        suggestions.push(AssessmentSuggestion {
            finding_id: "ambient.posture-stable".into(),
            severity: "info".into(),
            title: "Ambient posture stable — maintain operational review".into(),
            remediation: format!(
                "Namespace `{ns_name}` on ambient Istio shows no blockers in this scan. \
                 Continue periodic assessment, monitor ztunnel/HBONE metrics, and validate L7 policy \
                 (AuthorizationPolicy, waypoints) after each platform upgrade."
            ),
        });
    }
}

fn append_hostname_inventory(
    ns_name: &str,
    mesh: Option<&MeshInstance>,
    workload_count: u32,
    findings: &mut Vec<Finding>,
    suggestions: &mut Vec<AssessmentSuggestion>,
) {
    if workload_count == 0 {
        return;
    }
    if findings
        .iter()
        .any(|f| f.id == "traffic.ambient-routing-inventory" || f.id == "traffic.missing-hostnames")
    {
        return;
    }
    let rev = mesh
        .map(|m| format!(" (revision `{}`)", m.revision))
        .unwrap_or_default();
    findings.push(Finding {
        id: "traffic.ambient-routing-inventory".into(),
        severity: FindingSeverity::Warning,
        category: FindingCategory::TrafficCompatibility,
        title: "External routing inventory incomplete".into(),
        message: format!(
            "Namespace `{ns_name}` runs on ambient Istio{rev} with active workloads but no \
             VirtualService or HTTPRoute hostnames were indexed. Without a documented routing \
             inventory, L7 cutover validation and incident triage are harder."
        ),
        namespace: Some(ns_name.to_string()),
        resource: None,
        remediation: Some(
            "Enumerate production hostnames on VirtualService `spec.hosts` or HTTPRoute \
             `spec.hostnames`, confirm Gateway/waypoint attachment, and attach evidence to the \
             change record."
                .into(),
        ),
        doc_url: Some(AMBIENT_MIGRATE_DOC.into()),
        evidence: None,
    });
    suggestions.push(AssessmentSuggestion {
        finding_id: "traffic.ambient-routing-inventory".into(),
        severity: "warning".into(),
        title: "Complete routing inventory for ambient operations".into(),
        remediation: format!(
            "For `{ns_name}`: export Gateway + HTTPRoute/VirtualService hostnames, verify DNS and \
             TLS termination paths, then re-run assessment to close this operational gap."
        ),
    });
}

fn append_workload_investigation(
    ns_name: &str,
    dataplane: DataplaneMode,
    ctx: &RuleContext,
    findings: &mut Vec<Finding>,
    suggestions: &mut Vec<AssessmentSuggestion>,
) {
    let workloads: Vec<_> = ctx
        .workloads
        .iter()
        .filter(|w| w.namespace == ns_name)
        .collect();

    if dataplane == DataplaneMode::Ambient {
        let sidecars: Vec<_> = workloads
            .iter()
            .filter(|w| w.has_istio_sidecar)
            .map(|w| w.name.as_str())
            .collect();
        if !sidecars.is_empty() && !findings.iter().any(|f| f.id == "ambient.residual-sidecars") {
            let sample: String = sidecars
                .iter()
                .take(5)
                .copied()
                .collect::<Vec<_>>()
                .join(", ");
            let more = sidecars.len().saturating_sub(5);
            let suffix = if more > 0 {
                format!(" (+{more} more)")
            } else {
                String::new()
            };
            findings.push(Finding {
                id: "ambient.residual-sidecars".into(),
                severity: FindingSeverity::Warning,
                category: FindingCategory::SidecarDependency,
                title: "Residual sidecar proxies detected".into(),
                message: format!(
                    "Namespace `{ns_name}` is ambient-labeled but pods still run Istio sidecars \
                     ({sample}{suffix}). Mixed dataplanes complicate policy and observability."
                ),
                namespace: Some(ns_name.to_string()),
                resource: None,
                remediation: Some(
                    "Identify workloads that require sidecars (VM, hold-until-proxy, custom Envoy) \
                     and plan removal or isolation; verify ztunnel capture for remaining pods."
                        .into(),
                ),
                doc_url: Some(AMBIENT_MIGRATE_DOC.into()),
                evidence: Some(format!("pods: {sample}{suffix}")),
            });
            suggestions.push(deep_suggestion(
                "ambient.residual-sidecars",
                "warning",
                "Investigate mixed dataplane pods",
                &format!(
                    "Audit sidecar pods in `{ns_name}`: confirm injection labels, rollout status, \
                     and whether ambient-only traffic is intended."
                ),
            ));
        }
    }

    for w in workloads {
        if w.uses_localhost_proxy
            && !findings.iter().any(|f| {
                f.id == "sidecar.localhost-proxy" && f.evidence.as_deref() == Some(&w.name)
            })
        {
            let hits = w.localhost_proxy_hits.join("; ");
            findings.push(Finding {
                id: "sidecar.localhost-proxy".into(),
                severity: FindingSeverity::Warning,
                category: FindingCategory::SidecarDependency,
                title: "Localhost proxy dependency".into(),
                message: format!(
                    "Pod `{}/{}` references localhost proxy ports — common blocker for ambient HBONE.",
                    w.namespace, w.name
                ),
                namespace: Some(ns_name.to_string()),
                resource: Some(format!("{}/{}", w.namespace, w.name)),
                remediation: Some(
                    "Refactor app-to-proxy communication to service names or document an \
                     exception with a dedicated waypoint/sidecar retention plan."
                        .into(),
                ),
                doc_url: Some(AMBIENT_MIGRATE_DOC.into()),
                evidence: Some(format!("pod: {}\nhits: {}", w.name, hits)),
            });
            suggestions.push(deep_suggestion(
                "sidecar.localhost-proxy",
                "warning",
                "Deep dive: localhost proxy usage",
                &format!(
                    "Review container env and annotations on `{}/{}` ({hits}).",
                    w.namespace, w.name
                ),
            ));
        }
        if w.hold_until_proxy {
            findings.push(Finding {
                id: "sidecar.hold-until-proxy".into(),
                severity: FindingSeverity::Warning,
                category: FindingCategory::SidecarDependency,
                title: "holdUntilProxyActive blocks ambient cutover".into(),
                message: format!(
                    "Pod `{}/{}` uses `proxy.istio.io/config: holdUntilProxyActive` — workloads \
                     will not start cleanly without a sidecar.",
                    w.namespace, w.name
                ),
                namespace: Some(ns_name.to_string()),
                resource: Some(format!("{}/{}", w.namespace, w.name)),
                remediation: Some(
                    "Remove holdUntilProxyActive or keep the workload on sidecar dataplane with \
                     documented exception."
                        .into(),
                ),
                doc_url: Some(AMBIENT_MIGRATE_DOC.into()),
                evidence: Some(w.name.clone()),
            });
            suggestions.push(deep_suggestion(
                "sidecar.hold-until-proxy",
                "warning",
                "Resolve holdUntilProxyActive before ambient-only operation",
                &format!(
                    "Pod `{}/{}` must be refactored or excluded from ambient-only posture.",
                    w.namespace, w.name
                ),
            ));
        }
    }

    if let Some(ns_ctx) = ctx.namespaces.iter().find(|n| n.name == ns_name)
        && ns_ctx.has_vm_workloads
        && !findings.iter().any(|f| f.id == "readiness.vm-workloads")
    {
        findings.push(Finding {
            id: "readiness.vm-workloads".into(),
            severity: FindingSeverity::Warning,
            category: FindingCategory::Readiness,
            title: "VM workloads require extended validation".into(),
            message: format!(
                "Namespace `{ns_name}` includes VM/bare-metal workloads. Validate ztunnel \
                 onboarding, workload entry, and east-west policy separately from pod-only apps."
            ),
            namespace: Some(ns_name.to_string()),
            resource: None,
            remediation: Some(
                "Follow Istio ambient VM onboarding guides; verify WorkloadEntry and identity."
                    .into(),
            ),
            doc_url: Some(AMBIENT_MIGRATE_DOC.into()),
            evidence: None,
        });
    }
}

fn append_policy_investigation(
    ns_name: &str,
    ctx: &RuleContext,
    findings: &mut Vec<Finding>,
    suggestions: &mut Vec<AssessmentSuggestion>,
) {
    let mut policy_hits = BTreeSet::new();
    for name in &ctx.policies.l7_authorization_policies {
        if name.starts_with(&format!("{ns_name}/")) {
            policy_hits.insert(("L7 AuthorizationPolicy", name.clone()));
        }
    }
    for name in &ctx.policies.destination_rules_with_subsets {
        if name.starts_with(&format!("{ns_name}/")) {
            policy_hits.insert(("DestinationRule subsets", name.clone()));
        }
    }
    for name in &ctx.policies.envoy_filters {
        if name.starts_with(&format!("{ns_name}/")) {
            policy_hits.insert(("EnvoyFilter", name.clone()));
        }
    }

    if policy_hits.is_empty() {
        return;
    }

    let summary: Vec<String> = policy_hits
        .iter()
        .take(6)
        .map(|(kind, res)| format!("{kind}: {res}"))
        .collect();
    let more = policy_hits.len().saturating_sub(6);
    let tail = if more > 0 {
        format!(" (+{more} additional)")
    } else {
        String::new()
    };

    if findings
        .iter()
        .any(|f| f.id == "policy.ambient-translation-review")
    {
        return;
    }

    findings.push(Finding {
        id: "policy.ambient-translation-review".into(),
        severity: FindingSeverity::Info,
        category: FindingCategory::PolicyTranslation,
        title: "Policy objects require ambient translation review".into(),
        message: format!(
            "Namespace `{ns_name}` contains legacy or L7 policy objects that should be validated \
             under ambient: {}{tail}",
            summary.join("; "),
        ),
        namespace: Some(ns_name.to_string()),
        resource: None,
        remediation: Some(
            "Walk each policy through Istio ambient policy migration guidance; test on a \
             staging revision before production."
                .into(),
        ),
        doc_url: Some(AMBIENT_MIGRATE_DOC.into()),
        evidence: Some(summary.join("\n")),
    });
    suggestions.push(deep_suggestion(
        "policy.ambient-translation-review",
        "info",
        "Deep dive: policy translation for ambient",
        &format!(
            "Review listed policies in `{ns_name}` for waypoint/L7 compatibility and PeerAuthentication equivalents."
        ),
    ));
}

fn append_namespace_rule_synthesis(
    ns_name: &str,
    findings: &[Finding],
    suggestions: &mut Vec<AssessmentSuggestion>,
) {
    let existing: BTreeSet<String> = suggestions.iter().map(|s| s.finding_id.clone()).collect();

    for f in findings {
        if f.namespace.as_deref() != Some(ns_name) {
            continue;
        }
        if f.id.starts_with("traffic.ambient-routing-inventory")
            || f.id.starts_with("ambient.posture-stable")
            || existing.contains(&f.id)
        {
            continue;
        }
        let severity = format!("{:?}", f.severity).to_lowercase();
        let remediation = f.remediation.clone().unwrap_or_else(|| {
            "Review finding evidence, reproduce in staging, and document resolution.".into()
        });
        suggestions.push(deep_suggestion(
            &f.id,
            &severity,
            &format!("Investigate: {}", f.title),
            &remediation,
        ));
    }
}

fn deep_suggestion(
    finding_id: &str,
    severity: &str,
    title: &str,
    remediation: &str,
) -> AssessmentSuggestion {
    AssessmentSuggestion {
        finding_id: finding_id.to_string(),
        severity: severity.to_string(),
        title: title.to_string(),
        remediation: remediation.to_string(),
    }
}

fn has_open_issues(findings: &[Finding]) -> bool {
    findings.iter().any(|f| {
        matches!(
            f.severity,
            FindingSeverity::Blocker | FindingSeverity::Warning
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambientor_types::{MeshEnrollment, MeshEnrollmentMode};

    fn ambient_mesh() -> MeshInstance {
        MeshInstance {
            revision: "ambient-v1".into(),
            discovery_label: "mesh-ambient".into(),
            control_plane_namespace: "ambient-istio-system".into(),
            version: None,
            ambient: true,
            enrolled_namespace_count: 1,
            enrollment: MeshEnrollment {
                mode: MeshEnrollmentMode::DiscoveryLabel,
                revision: "ambient-v1".into(),
                istio_revision: Some("ambient-v1".into()),
                revision_tag: None,
                discovery_label_key: Some("istio-discovery".into()),
                discovery_label_value: Some("mesh-ambient".into()),
                member_roll_namespace: None,
                from_istiod_config: false,
            },
        }
    }

    #[test]
    fn skips_enrollment_nag_on_ambient_istiod() {
        let mesh = ambient_mesh();
        let mut findings = vec![Finding {
            id: "enrollment.missing-ambient-dataplane".into(),
            severity: FindingSeverity::Warning,
            category: FindingCategory::SidecarDependency,
            title: "should remove".into(),
            message: String::new(),
            namespace: Some("bookinfo".into()),
            resource: None,
            remediation: None,
            doc_url: None,
            evidence: None,
        }];
        let mut suggestions = Vec::new();
        enrich_ambient_application(
            "bookinfo",
            DataplaneMode::Sidecar,
            Some(&mesh),
            &RuleContext::default(),
            &mut findings,
            &mut suggestions,
            true,
            5,
        );
        assert!(
            !findings
                .iter()
                .any(|f| f.id == "enrollment.missing-ambient-dataplane")
        );
        assert!(
            findings
                .iter()
                .any(|f| f.id == "traffic.ambient-routing-inventory")
        );
    }
}
