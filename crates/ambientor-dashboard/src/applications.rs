use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use ambientor_core::rules::RuleContext;
use ambientor_core::scoring::compute_scores;
use ambientor_mesh::application_identity::NamespaceApplicationIdentity;
use ambientor_mesh::istio::collect_istio_policies;
use ambientor_mesh::is_application_namespace;
use ambientor_types::{
    Finding, FindingCategory, FindingSeverity, MeshInstance,
};
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::api::ListParams;
use kube::{Api, Client};

use crate::application_types::{
    ApplicationAssessmentRecord, AssessmentSuggestion, ClusterAssessmentRun, RiskLevel,
};
use crate::compute::namespace_belongs_to_mesh;
use crate::dataplane::{derive_dataplane_mode, is_ambient_mesh_scope, is_migration_candidate};
use crate::deep_analysis::enrich_ambient_application;
use crate::findings_attribution::partition_findings_by_namespace;

const ISTIO_LABEL_KEYS: &[&str] = &[
    "istio-discovery",
    "istio.io/rev",
    "istio-injection",
    "istio.io/dataplane-mode",
    "istio.io/use-waypoint",
];

pub async fn list_namespaces_for_assessment(client: &Client) -> anyhow::Result<Vec<Namespace>> {
    let api: Api<Namespace> = Api::all(client.clone());
    Ok(api.list(&ListParams::default()).await?.items)
}

pub async fn discover_ingress_gateway_namespaces(client: &Client) -> anyhow::Result<HashSet<String>> {
    let api: Api<Pod> = Api::all(client.clone());
    let pods = api.list(&ListParams::default()).await?.items;
    let mut set = HashSet::new();
    for pod in pods {
        let labels = pod.metadata.labels.as_ref();
        let is_gateway = labels.is_some_and(|l| {
            l.get("app")
                .is_some_and(|v| v.contains("ingressgateway") || v == "istio-ingressgateway")
                || l.get("istio")
                    .is_some_and(|v| v == "ingressgateway" || v.contains("ingress"))
        });
        if is_gateway
            && let Some(ns) = pod.metadata.namespace
        {
            set.insert(ns);
        }
    }
    Ok(set)
}

pub async fn hostnames_by_namespace(client: &Client) -> anyhow::Result<HashMap<String, BTreeSet<String>>> {
    let objects = collect_istio_policies(client).await?;
    let mut by_namespace: HashMap<String, BTreeSet<String>> = HashMap::new();
    for vs in &objects.virtual_services {
        let ns = vs.metadata.namespace.clone().unwrap_or_default();
        if let Some(hosts) = vs
            .data
            .get("spec")
            .and_then(|s| s.get("hosts"))
            .and_then(|h| h.as_array())
        {
            for host in hosts {
                if let Some(s) = host.as_str() {
                    by_namespace
                        .entry(ns.clone())
                        .or_default()
                        .insert(s.to_string());
                }
            }
        }
    }
    for hr in &objects.http_routes {
        let ns = hr.metadata.namespace.clone().unwrap_or_default();
        if let Some(hosts) = hr
            .data
            .get("spec")
            .and_then(|s| s.get("hostnames"))
            .and_then(|h| h.as_array())
        {
            for host in hosts {
                if let Some(s) = host.as_str() {
                    by_namespace
                        .entry(ns.clone())
                        .or_default()
                        .insert(s.to_string());
                }
            }
        }
    }
    Ok(by_namespace)
}

pub fn build_cluster_assessment(
    cluster_ref: &str,
    ctx: &RuleContext,
    all_findings: &[Finding],
    namespaces: &[Namespace],
    mesh_instances: &[MeshInstance],
    hostnames_by_ns: &HashMap<String, BTreeSet<String>>,
    ingress_ns: &HashSet<String>,
    identities: &std::collections::BTreeMap<String, NamespaceApplicationIdentity>,
) -> ClusterAssessmentRun {
    let (by_ns, cluster_findings) = partition_findings_by_namespace(all_findings, ctx);

    let mut applications = Vec::new();
    let mut seen = HashSet::new();

    for mesh in mesh_instances {
        for ns in namespaces {
            let Some(ns_name) = ns.metadata.name.clone() else {
                continue;
            };
            if !is_application_namespace(&ns_name, mesh_instances) {
                continue;
            }
            if !namespace_belongs_to_mesh(ns.metadata.labels.as_ref(), mesh) {
                continue;
            }
            if !seen.insert(ns_name.clone()) {
                continue;
            }
            let findings = by_ns.get(&ns_name).cloned().unwrap_or_default();
            applications.push(build_app_record(
                ctx,
                &ns_name,
                ns.metadata.labels.as_ref(),
                Some(mesh),
                &findings,
                hostnames_by_ns,
                ingress_ns,
                identities.get(&ns_name),
            ));
        }
    }

    for (ns_name, findings) in &by_ns {
        if seen.contains(ns_name) || !is_application_namespace(ns_name, mesh_instances) {
            continue;
        }
        let ns_obj = namespaces
            .iter()
            .find(|n| n.metadata.name.as_deref() == Some(ns_name.as_str()));
        let labels = ns_obj.and_then(|n| n.metadata.labels.as_ref());
        let mesh = mesh_instances
            .iter()
            .find(|m| namespace_belongs_to_mesh(labels, m));
        if mesh.is_none() && findings.is_empty() {
            continue;
        }
        seen.insert(ns_name.clone());
        applications.push(build_app_record(
            ctx,
            ns_name,
            labels,
            mesh,
            findings,
            hostnames_by_ns,
            ingress_ns,
            identities.get(ns_name),
        ));
    }

    applications.sort_by(|a, b| {
        a.application_name
            .cmp(&b.application_name)
            .then(a.namespace.cmp(&b.namespace))
    });

    ClusterAssessmentRun {
        cluster_ref: cluster_ref.to_string(),
        applications,
        cluster_scores: compute_scores(all_findings),
        cluster_summary: ambientor_types::FindingSummary::from_findings(all_findings),
        cluster_findings,
    }
}

fn build_app_record(
    ctx: &RuleContext,
    ns_name: &str,
    labels: Option<&BTreeMap<String, String>>,
    mesh: Option<&MeshInstance>,
    findings: &[Finding],
    hostnames_by_ns: &HashMap<String, BTreeSet<String>>,
    ingress_ns: &HashSet<String>,
    identity: Option<&NamespaceApplicationIdentity>,
) -> ApplicationAssessmentRecord {
    let app_pod_count = identity.map(|i| i.app_pod_count).unwrap_or_else(|| {
        workload_count(ctx, ns_name)
    });
    let application_name = identity
        .map(|i| i.application_name.clone())
        .unwrap_or_else(|| ns_name.to_string());
    let workload_components = identity
        .map(|i| i.workload_components.clone())
        .unwrap_or_default();
    let hostnames: Vec<String> = hostnames_by_ns
        .get(ns_name)
        .map(|s| s.iter().cloned().collect())
        .unwrap_or_default();

    let label_map = labels.cloned().unwrap_or_default();
    let dataplane = derive_dataplane_mode(&label_map, mesh);

    let ambient_scope = is_ambient_mesh_scope(dataplane, mesh);
    let hostnames_empty = hostnames.is_empty();

    let mut app_findings = findings.to_vec();
    if hostnames_empty
        && should_warn_missing_hostnames(mesh, labels, app_pod_count)
        && !ambient_scope
    {
        app_findings.push(missing_hostname_finding(ns_name, mesh));
    }

    let mut suggestions = if ambient_scope {
        Vec::new()
    } else {
        suggestions_from_findings(&app_findings)
    };
    if hostnames_empty
        && should_warn_missing_hostnames(mesh, labels, app_pod_count)
        && !ambient_scope
    {
        suggestions.push(missing_hostname_suggestion(ns_name));
    }

    if ambient_scope {
        enrich_ambient_application(
            ns_name,
            dataplane,
            mesh,
            ctx,
            &mut app_findings,
            &mut suggestions,
            hostnames_empty,
            app_pod_count,
        );
    }

    let migration_candidate =
        is_migration_candidate(dataplane, app_pod_count, &label_map, mesh);

    let scores = compute_scores(&app_findings);
    let summary = ambientor_types::FindingSummary::from_findings(&app_findings);
    let readiness_pct = scores.overall;
    let risk_level = derive_risk_level(&summary, readiness_pct);

    let ingress_gateway_namespace = if ingress_ns.contains(ns_name) {
        Some(ns_name.to_string())
    } else {
        ingress_ns.iter().min().cloned()
    };
    let ingress_same_namespace = ingress_ns.contains(ns_name);

    ApplicationAssessmentRecord {
        namespace: ns_name.to_string(),
        application_name,
        workload_components,
        migration_candidate,
        mesh_revision: mesh.map(|m| m.revision.clone()),
        discovery_label: mesh.map(|m| m.discovery_label.clone()),
        control_plane_namespace: mesh.map(|m| m.control_plane_namespace.clone()),
        hostnames,
        namespace_labels: istio_namespace_labels(labels),
        dataplane_mode: dataplane.as_str().to_string(),
        ingress_gateway_namespace,
        ingress_same_namespace,
        workload_count: app_pod_count,
        readiness_pct,
        risk_level,
        blocker_count: summary.blockers,
        warning_count: summary.warnings,
        scores,
        summary,
        findings: app_findings,
        suggestions,
    }
}

fn should_warn_missing_hostnames(
    mesh: Option<&MeshInstance>,
    labels: Option<&BTreeMap<String, String>>,
    workload_count: u32,
) -> bool {
    if mesh.is_none() {
        return false;
    }
    if workload_count == 0 {
        return false;
    }
    let Some(labels) = labels else {
        return true;
    };
    labels.contains_key("istio.io/rev")
        || labels.contains_key("istio-discovery")
        || labels
            .get("istio-injection")
            .is_some_and(|v| v == "enabled" || v == "true")
}

fn missing_hostname_finding(ns_name: &str, mesh: Option<&MeshInstance>) -> Finding {
    let mesh_hint = mesh
        .map(|m| format!(" (istiod revision `{}`)", m.revision))
        .unwrap_or_default();
    Finding {
        id: "traffic.missing-hostnames".into(),
        severity: FindingSeverity::Warning,
        category: FindingCategory::TrafficCompatibility,
        title: "No routable hostnames detected".into(),
        message: format!(
            "Namespace `{ns_name}` has workloads on the mesh{mesh_hint} but no VirtualService or HTTPRoute hostnames were found. External or east-west traffic may be undefined for ambient migration planning."
        ),
        namespace: Some(ns_name.to_string()),
        resource: None,
        remediation: Some(
            "Document public hostnames on VirtualService `spec.hosts` or Gateway API HTTPRoute `spec.hostnames`, or confirm the app is internal-only (service DNS only).".into(),
        ),
        doc_url: None,
        evidence: None,
    }
}

fn missing_hostname_suggestion(ns_name: &str) -> AssessmentSuggestion {
    AssessmentSuggestion {
        finding_id: "traffic.missing-hostnames".into(),
        severity: "warning".into(),
        title: "Declare hostnames before ambient cutover".into(),
        remediation: format!(
            "Add VirtualService/HTTPRoute hostnames for namespace `{ns_name}`, verify Gateway attachment, and re-run assessment. Missing hostnames increase traffic-cutover risk during migration."
        ),
    }
}

fn workload_count(ctx: &RuleContext, ns: &str) -> u32 {
    ctx.namespaces
        .iter()
        .find(|n| n.name == ns)
        .map(|n| n.workload_count)
        .unwrap_or(0)
}

pub fn derive_risk_level(summary: &ambientor_types::FindingSummary, readiness_pct: u8) -> RiskLevel {
    if summary.blockers > 0 {
        RiskLevel::Critical
    } else if readiness_pct < 50 {
        RiskLevel::High
    } else if summary.warnings > 0 || readiness_pct < 80 {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    }
}

fn suggestions_from_findings(findings: &[Finding]) -> Vec<AssessmentSuggestion> {
    findings
        .iter()
        .filter(|f| f.remediation.is_some())
        .map(|f| AssessmentSuggestion {
            finding_id: f.id.clone(),
            severity: format!("{:?}", f.severity).to_lowercase(),
            title: f.title.clone(),
            remediation: f.remediation.clone().unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn critical_risk_when_blockers() {
        let summary = ambientor_types::FindingSummary {
            blockers: 1,
            warnings: 0,
            info: 0,
        };
        assert_eq!(derive_risk_level(&summary, 90), RiskLevel::Critical);
    }
}

fn istio_namespace_labels(labels: Option<&BTreeMap<String, String>>) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let Some(labels) = labels else {
        return out;
    };
    for key in ISTIO_LABEL_KEYS {
        if let Some(v) = labels.get(*key) {
            out.insert((*key).to_string(), v.clone());
        }
    }
    out
}
