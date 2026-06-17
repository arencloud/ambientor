use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use ambientor_core::rules::RuleContext;
use ambientor_core::scoring::compute_scores;
use ambientor_mesh::application_identity::NamespaceApplicationIdentity;
use ambientor_mesh::policy_collect::IstioPolicyObjects;
use ambientor_mesh::{is_ambient_control_plane_namespace, is_application_namespace, is_mesh_infrastructure_identity};
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

pub fn ingress_gateway_namespaces_from_pods(pods: &[Pod]) -> HashSet<String> {
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
            && let Some(ns) = pod.metadata.namespace.as_deref()
        {
            set.insert(ns.to_string());
        }
    }
    set
}

pub async fn discover_ingress_gateway_namespaces(client: &Client) -> anyhow::Result<HashSet<String>> {
    let api: Api<Pod> = Api::all(client.clone());
    let pods = api.list(&ListParams::default()).await?.items;
    Ok(ingress_gateway_namespaces_from_pods(&pods))
}

pub fn hostnames_from_istio_objects(
    objects: &IstioPolicyObjects,
) -> HashMap<String, BTreeSet<String>> {
    let mut by_namespace: HashMap<String, BTreeSet<String>> = HashMap::new();
    for vs in objects.virtual_services.iter() {
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
    for hr in objects.http_routes.iter() {
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
    by_namespace
}

pub async fn hostnames_by_namespace(client: &Client) -> anyhow::Result<HashMap<String, BTreeSet<String>>> {
    use ambientor_mesh::istio::collect_istio_policies;
    let objects = collect_istio_policies(client).await?;
    Ok(hostnames_from_istio_objects(&objects))
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
            if !namespace_eligible_for_catalog(&ns_name, mesh_instances, identities) {
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
                mesh_instances,
            ));
        }
    }

    for (ns_name, findings) in &by_ns {
        if seen.contains(ns_name)
            || !namespace_eligible_for_catalog(ns_name, mesh_instances, identities)
        {
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
            mesh_instances,
        ));
    }

    // Namespaces with sidecar workloads that may lack mesh labels but still need migration.
    for ns in namespaces {
        let Some(ns_name) = ns.metadata.name.clone() else {
            continue;
        };
        if !namespace_eligible_for_catalog(&ns_name, mesh_instances, identities)
            || seen.contains(&ns_name)
        {
            continue;
        }
        let identity = identities.get(&ns_name);
        let app_pods = identity.map(|i| i.app_pod_count).unwrap_or(0);
        if app_pods == 0 {
            continue;
        }
        let has_sidecar = ctx.workloads.iter().any(|w| {
            w.namespace == ns_name && w.has_istio_sidecar
        });
        if !has_sidecar {
            continue;
        }
        seen.insert(ns_name.clone());
        let findings = by_ns.get(&ns_name).cloned().unwrap_or_default();
        let mesh = mesh_instances
            .iter()
            .find(|m| namespace_belongs_to_mesh(ns.metadata.labels.as_ref(), m));
        applications.push(build_app_record(
            ctx,
            &ns_name,
            ns.metadata.labels.as_ref(),
            mesh,
            &findings,
            hostnames_by_ns,
            ingress_ns,
            identity,
            mesh_instances,
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
    mesh_instances: &[MeshInstance],
) -> ApplicationAssessmentRecord {
    let app_pod_count = identity.map(|i| i.app_pod_count).unwrap_or(0);
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

    let migration_candidate = is_migration_candidate(
        ns_name,
        dataplane,
        app_pod_count,
        &label_map,
        mesh,
        mesh_instances,
        identity,
    );

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

fn namespace_eligible_for_catalog(
    ns_name: &str,
    mesh_instances: &[MeshInstance],
    identities: &std::collections::BTreeMap<String, NamespaceApplicationIdentity>,
) -> bool {
    if !is_application_namespace(ns_name, mesh_instances)
        || is_ambient_control_plane_namespace(ns_name, mesh_instances)
    {
        return false;
    }
    match identities.get(ns_name) {
        Some(id) if is_mesh_infrastructure_identity(id) => false,
        Some(id) if id.app_pod_count == 0 => false,
        Some(_) => true,
        None => false,
    }
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
    use ambientor_core::rules::RuleContext;
    use ambientor_mesh::application_identity::identities_by_namespace;
    use ambientor_types::{MeshEnrollment, MeshEnrollmentMode, MeshInstance};
    use k8s_openapi::api::core::v1::{Container, Pod, PodSpec};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    #[test]
    fn critical_risk_when_blockers() {
        let summary = ambientor_types::FindingSummary {
            blockers: 1,
            warnings: 0,
            info: 0,
        };
        assert_eq!(derive_risk_level(&summary, 90), RiskLevel::Critical);
    }

    #[test]
    fn ztunnel_not_migration_candidate() {
        let mesh = MeshInstance {
            revision: "ambient-v1-28-6".into(),
            discovery_label: "mesh-ambient".into(),
            control_plane_namespace: "ambient-v1-28-6-istio-system".into(),
            version: Some("1.28.6".into()),
            ambient: true,
            enrolled_namespace_count: 0,
            enrollment: MeshEnrollment {
                mode: MeshEnrollmentMode::RevisionAndDiscovery,
                revision: "ambient-v1-28-6".into(),
                istio_revision: Some("ambient-v1-28-6".into()),
                revision_tag: None,
                discovery_label_key: Some("istio-discovery".into()),
                discovery_label_value: Some("mesh-ambient".into()),
                member_roll_namespace: None,
                from_istiod_config: false,
            },
        };
        let ns = Namespace {
            metadata: ObjectMeta {
                name: Some("ambient-v1-28-6-istio-system".into()),
                labels: Some(BTreeMap::from([(
                    "istio-discovery".into(),
                    "mesh-ambient".into(),
                )])),
                ..Default::default()
            },
            ..Default::default()
        };
        let ztunnel = Pod {
            metadata: ObjectMeta {
                namespace: Some("ambient-v1-28-6-istio-system".into()),
                name: Some("ztunnel-abc".into()),
                labels: Some(BTreeMap::from([(
                    "app.kubernetes.io/name".into(),
                    "ztunnel".into(),
                )])),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "ztunnel".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let ctx = RuleContext {
            mesh_version: None,
            mesh_flavor: None,
            ambient_installed: true,
            gateway_api_present: false,
            namespaces: vec![],
            workloads: vec![],
            policies: Default::default(),
            platform: Default::default(),
        };
        let run = build_cluster_assessment(
            "ambientor-system/cl02",
            &ctx,
            &[],
            &[ns],
            &[mesh],
            &HashMap::new(),
            &HashSet::new(),
            &identities_by_namespace(&[ztunnel]),
        );
        assert!(
            run.applications
                .iter()
                .all(|a| !a.migration_candidate),
            "expected no migration candidates, got {:?}",
            run
                .applications
                .iter()
                .filter(|a| a.migration_candidate)
                .map(|a| &a.application_name)
                .collect::<Vec<_>>()
        );
        assert!(
            run.applications.is_empty(),
            "infra-only namespace should not appear in catalog, got {:?}",
            run.applications
                .iter()
                .map(|a| &a.namespace)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn ztunnel_only_enrolled_namespace_not_in_catalog() {
        let mesh = MeshInstance {
            revision: "ambient-v1-28-6".into(),
            discovery_label: "mesh-ambient".into(),
            control_plane_namespace: "ambient-v1-28-6-istio-system".into(),
            version: Some("1.28.6".into()),
            ambient: true,
            enrolled_namespace_count: 1,
            enrollment: MeshEnrollment {
                mode: MeshEnrollmentMode::RevisionAndDiscovery,
                revision: "ambient-v1-28-6".into(),
                istio_revision: Some("ambient-v1-28-6".into()),
                revision_tag: None,
                discovery_label_key: Some("istio-discovery".into()),
                discovery_label_value: Some("mesh-ambient".into()),
                member_roll_namespace: None,
                from_istiod_config: false,
            },
        };
        // Non-standard namespace name (not *-istio-system) with only ztunnel pods.
        let ns = Namespace {
            metadata: ObjectMeta {
                name: Some("mesh-dataplane".into()),
                labels: Some(BTreeMap::from([(
                    "istio-discovery".into(),
                    "mesh-ambient".into(),
                )])),
                ..Default::default()
            },
            ..Default::default()
        };
        let ztunnel = Pod {
            metadata: ObjectMeta {
                namespace: Some("mesh-dataplane".into()),
                name: Some("ztunnel-node-1".into()),
                labels: Some(BTreeMap::from([("app".into(), "ztunnel".into())])),
                ..Default::default()
            },
            spec: Some(PodSpec {
                containers: vec![Container {
                    name: "ztunnel".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }),
            ..Default::default()
        };
        let ctx = RuleContext {
            mesh_version: None,
            mesh_flavor: None,
            ambient_installed: true,
            gateway_api_present: false,
            namespaces: vec![ambientor_core::rules::NamespaceContext {
                name: "mesh-dataplane".into(),
                injection_enabled: false,
                ambient_enabled: false,
                workload_count: 1,
                has_vm_workloads: false,
            }],
            workloads: vec![],
            policies: Default::default(),
            platform: Default::default(),
        };
        let run = build_cluster_assessment(
            "ambientor-system/cl02",
            &ctx,
            &[],
            &[ns],
            &[mesh],
            &HashMap::new(),
            &HashSet::new(),
            &identities_by_namespace(&[ztunnel]),
        );
        assert!(
            run.applications.is_empty(),
            "ztunnel-only namespace must not be cataloged: {:?}",
            run.applications
        );
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
