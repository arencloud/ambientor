use std::collections::{BTreeMap, BTreeSet, HashMap};

use ambientor_mesh::application_identity::identities_by_namespace;
use ambientor_mesh::mesh_instances::discover_mesh_instances;
use ambientor_mesh::{
    is_ambient_control_plane_namespace, is_application_namespace, is_mesh_infrastructure_identity,
    namespace_enrolled_on_mesh,
};
use k8s_openapi::api::core::v1::Pod;

use crate::dataplane::derive_dataplane_mode;
use ambientor_mesh::version::detect_istio_version;
use ambientor_types::{
    AmbientAssessment, Cluster, FindingSeverity, MeshInstance, MeshInventory, Rollout,
};
use k8s_openapi::api::core::v1::Namespace;
use kube::{Api, Client, api::ListParams};

use crate::types::{
    ApplicationMigrationStatus, ApplicationRow, ClusterDashboard, DashboardResponse,
    MeshInstanceDashboard, MigrationSavingsSummary, StatusCounts,
};

/// Findings keyed by `AmbientAssessment` name when CR status omits them (Postgres canonical).
pub type AssessmentFindingsOverrides = HashMap<String, Vec<ambientor_types::Finding>>;

pub async fn build_dashboard(
    client: &Client,
    cluster_ref: &str,
    findings_overrides: Option<&AssessmentFindingsOverrides>,
    rollout_client: Option<&Client>,
) -> anyhow::Result<DashboardResponse> {
    let platform = ambientor_k8s::detect_platform(client).await?;
    let istio_version = detect_istio_version(client).await;
    let cluster_name = load_cluster_display_name(client).await;

    let namespaces = list_namespaces(client).await?;
    let mesh_instances = discover_mesh_instances(client).await?;

    let assessments = list_assessments_map(client, findings_overrides).await?;
    let rollouts = list_rollout_ns_status(rollout_client.unwrap_or(client), cluster_ref).await?;
    let inventories = list_mesh_inventories(client).await?;
    let pod_api: Api<Pod> = Api::all(client.clone());
    let pods = pod_api.list(&ListParams::default()).await?.items;
    let identities = identities_by_namespace(&pods);

    let mut summary = StatusCounts::default();
    let mut mesh_dashboards = Vec::new();

    for mesh in &mesh_instances {
        let mut counts = StatusCounts::default();
        let mut apps = Vec::new();

        for ns in &namespaces {
            let Some(ns_name) = ns.metadata.name.clone() else {
                continue;
            };
            if !is_application_namespace(&ns_name, &mesh_instances) {
                continue;
            }
            if is_ambient_control_plane_namespace(&ns_name, &mesh_instances) {
                continue;
            }
            let labels = ns.metadata.labels.as_ref();
            if !namespace_belongs_to_mesh(labels, mesh) {
                continue;
            }
            let identity = identities.get(&ns_name);
            if identity.is_none_or(|id| id.app_pod_count == 0) {
                continue;
            }
            if identity.is_some_and(is_mesh_infrastructure_identity) {
                continue;
            }

            let status = derive_status(
                &ns_name,
                labels,
                mesh,
                &assessments,
                &rollouts,
                &inventories,
            );
            let label_map = labels.cloned().unwrap_or_default();
            let dataplane = derive_dataplane_mode(&label_map, Some(mesh));

            let blocker_count = assessments
                .get(&ns_name)
                .map(|a| {
                    a.findings
                        .iter()
                        .filter(|f| matches!(f.severity, FindingSeverity::Blocker))
                        .count()
                })
                .unwrap_or(0);

            let application_name = identity
                .map(|i| i.application_name.clone())
                .unwrap_or_else(|| ns_name.clone());

            let workload_count = identity.map(|i| i.app_pod_count).unwrap_or(0);

            let row = ApplicationRow {
                application_name,
                namespace: ns_name.clone(),
                status,
                mesh_revision: mesh.revision.clone(),
                discovery_label: mesh.discovery_label.clone(),
                dataplane_mode: dataplane.as_str().to_string(),
                ambient_dataplane: dataplane.is_ambient(),
                blocker_count,
                workload_count,
                rollout_phase: rollouts.get(&ns_name).cloned(),
                assessment_ref: assessments.get(&ns_name).map(|a| a.name.clone()),
            };

            increment_count(&mut counts, status);
            apps.push(row);
        }

        apps.sort_by(|a, b| {
            a.application_name
                .cmp(&b.application_name)
                .then(a.namespace.cmp(&b.namespace))
        });
        counts.total = apps.len();
        aggregate_counts(&mut summary, &counts);

        mesh_dashboards.push(MeshInstanceDashboard {
            revision: mesh.revision.clone(),
            discovery_label: mesh.discovery_label.clone(),
            control_plane_namespace: mesh.control_plane_namespace.clone(),
            version: mesh.version.clone(),
            ambient: mesh.ambient,
            counts,
            applications: apps,
        });
    }

    mesh_dashboards.sort_by(|a, b| {
        b.ambient
            .cmp(&a.ambient)
            .then(a.discovery_label.cmp(&b.discovery_label))
    });
    summary.total = summary.migrated
        + summary.processing
        + summary.blocker
        + summary.failed
        + summary.scanned
        + summary.not_scanned;

    let migration_savings = Some(compute_migration_savings_from_dashboard(&mesh_dashboards));

    Ok(DashboardResponse {
        cluster_ref: cluster_ref.to_string(),
        cluster: ClusterDashboard {
            name: cluster_name,
            platform: if platform.is_openshift {
                "OpenShift".into()
            } else {
                "Kubernetes".into()
            },
            mesh_flavor: format!("{:?}", platform.mesh_flavor),
            istio_version,
            mesh_instance_count: mesh_instances.len(),
            ambient_mesh_count: mesh_instances.iter().filter(|m| m.ambient).count(),
        },
        summary,
        mesh_instances: mesh_dashboards,
        migration_savings,
        last_updated: chrono::Utc::now().to_rfc3339(),
        connection_namespace: None,
        connection_name: None,
        reachable: None,
        is_hub: None,
    })
}

/// Estimated resource savings from workloads on ambient dataplane (dashboard **Migrated**).
pub fn compute_migration_savings_from_dashboard(
    meshes: &[MeshInstanceDashboard],
) -> MigrationSavingsSummary {
    let migrated_workloads: u32 = meshes
        .iter()
        .flat_map(|m| m.applications.iter())
        .filter(|a| a.status == ApplicationMigrationStatus::Migrated)
        .map(|a| a.workload_count.max(1))
        .sum();
    MigrationSavingsSummary {
        migrated_workloads,
        estimated_sidecar_proxies_removed: migrated_workloads,
        estimated_memory_mib_saved: migrated_workloads.saturating_mul(128),
        estimated_cpu_millicores_saved: migrated_workloads.saturating_mul(100),
    }
}

pub fn namespace_belongs_to_mesh(
    labels: Option<&BTreeMap<String, String>>,
    mesh: &MeshInstance,
) -> bool {
    let Some(labels) = labels else {
        return false;
    };
    if namespace_enrolled_on_mesh(labels, mesh) {
        return true;
    }
    if let Some(rev) = labels.get("istio.io/rev")
        && rev == &mesh.revision
    {
        return true;
    }
    if let Some(d) = labels.get("istio-discovery")
        && d == &mesh.discovery_label
    {
        return true;
    }
    false
}

fn derive_status(
    ns: &str,
    labels: Option<&BTreeMap<String, String>>,
    mesh: &MeshInstance,
    assessments: &HashMap<String, AssessmentNsInfo>,
    rollouts: &HashMap<String, String>,
    inventories: &BTreeSet<String>,
) -> ApplicationMigrationStatus {
    if let Some(phase) = rollouts.get(ns) {
        return match phase.as_str() {
            "Failed" | "RolledBack" => ApplicationMigrationStatus::Failed,
            "Completed" if mesh.ambient && is_migrated(labels) => ApplicationMigrationStatus::Migrated,
            "Completed" => ApplicationMigrationStatus::Scanned,
            "Running" | "AwaitingApproval" | "Pending" => ApplicationMigrationStatus::Processing,
            _ => ApplicationMigrationStatus::Processing,
        };
    }

    if mesh.ambient && is_migrated(labels) {
        return ApplicationMigrationStatus::Migrated;
    }

    if let Some(info) = assessments.get(ns) {
        if info.has_blocker {
            return ApplicationMigrationStatus::Blocker;
        }
        if inventories.contains(ns) || info.scanned {
            return ApplicationMigrationStatus::Scanned;
        }
    }

    if inventories.contains(ns) {
        return ApplicationMigrationStatus::Scanned;
    }

    ApplicationMigrationStatus::NotScanned
}

fn is_migrated(labels: Option<&BTreeMap<String, String>>) -> bool {
    labels
        .and_then(|l| l.get("istio.io/dataplane-mode"))
        .map(String::as_str)
        == Some("ambient")
}

struct AssessmentNsInfo {
    name: String,
    findings: Vec<ambientor_types::Finding>,
    has_blocker: bool,
    scanned: bool,
}

async fn list_assessments_map(
    client: &Client,
    findings_overrides: Option<&AssessmentFindingsOverrides>,
) -> anyhow::Result<HashMap<String, AssessmentNsInfo>> {
    let api: Api<AmbientAssessment> = Api::all(client.clone());
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(kube::Error::Api(e)) if e.code == 404 => return Ok(HashMap::new()),
        Err(e) => return Err(e.into()),
    };
    let mut map = HashMap::new();

    for a in list.items {
        let Some(status) = a.status.as_ref() else {
            continue;
        };
        let name = a
            .metadata
            .name
            .clone()
            .unwrap_or_else(|| "unknown".into());
        let scanned = status.phase == "Completed" || status.phase == "Ready";

        let findings = if status.findings.is_empty() {
            findings_overrides
                .and_then(|o| o.get(&name))
                .cloned()
                .unwrap_or_default()
        } else {
            status.findings.clone()
        };

        let mut ns_set: BTreeSet<String> = BTreeSet::new();
        for f in &findings {
            if let Some(ns) = &f.namespace {
                ns_set.insert(ns.clone());
            }
        }
        if ns_set.is_empty()
            && let Some(ns) = a.metadata.namespace.clone()
        {
            ns_set.insert(ns);
        }

        let has_blocker = findings
            .iter()
            .any(|f| matches!(f.severity, FindingSeverity::Blocker));

        for ns in ns_set {
            map.insert(
                ns,
                AssessmentNsInfo {
                    name: name.clone(),
                    findings: findings.clone(),
                    has_blocker,
                    scanned,
                },
            );
        }
    }
    Ok(map)
}

/// Hub Rollout CR phases keyed by target namespace (for overlaying cached assessment snapshots).
pub async fn list_rollout_ns_status(
    client: &Client,
    cluster_ref: &str,
) -> anyhow::Result<HashMap<String, String>> {
    let api: Api<Rollout> = Api::all(client.clone());
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(kube::Error::Api(e)) if e.code == 404 => return Ok(HashMap::new()),
        Err(e) => return Err(e.into()),
    };
    let mut map = HashMap::new();
    let hub_local = ambientor_k8s::parse_connection_cluster_ref(cluster_ref).is_none();

    for r in list.items {
        let rollout_cluster = r
            .spec
            .cluster_ref
            .as_deref()
            .filter(|s| !s.is_empty());
        let targets_cluster = match rollout_cluster {
            Some(rc) if !rc.is_empty() => rc == cluster_ref,
            _ => hub_local,
        };
        if !targets_cluster {
            continue;
        }
        let phase = r
            .status
            .as_ref()
            .map(|s| s.phase.clone())
            .unwrap_or_default();
        for stage in &r.spec.stages {
            for ns in &stage.namespaces {
                map.insert(ns.clone(), phase.clone());
            }
        }
    }
    Ok(map)
}

async fn list_mesh_inventories(client: &Client) -> anyhow::Result<BTreeSet<String>> {
    let api: Api<MeshInventory> = Api::all(client.clone());
    let list = match api.list(&ListParams::default()).await {
        Ok(l) => l,
        Err(kube::Error::Api(e)) if e.code == 404 => return Ok(BTreeSet::new()),
        Err(e) => return Err(e.into()),
    };
    let mut out = BTreeSet::new();
    for inv in list.items {
        if let Some(ns) = inv.metadata.namespace {
            out.insert(ns);
        }
    }
    Ok(out)
}

pub async fn load_cluster_display_name(client: &Client) -> String {
    if let Ok(name) = std::env::var("CLUSTER_NAME")
        && !name.is_empty()
    {
        return name;
    }
    let api: Api<Cluster> = Api::all(client.clone());
    if let Ok(list) = api.list(&ListParams::default()).await
        && let Some(c) = list.items.first()
    {
        if let Some(n) = c.spec.display_name.clone() {
            return n;
        }
        if let Some(name) = c.metadata.name.clone() {
            return name;
        }
    }
    std::env::var("POD_NAMESPACE")
        .ok()
        .map(|_| "Connected cluster".into())
        .unwrap_or_else(|| "Hub cluster".into())
}

async fn list_namespaces(client: &Client) -> anyhow::Result<Vec<Namespace>> {
    let api: Api<Namespace> = Api::all(client.clone());
    Ok(api.list(&ListParams::default()).await?.items)
}

fn increment_count(counts: &mut StatusCounts, status: ApplicationMigrationStatus) {
    match status {
        ApplicationMigrationStatus::Migrated => counts.migrated += 1,
        ApplicationMigrationStatus::Processing => counts.processing += 1,
        ApplicationMigrationStatus::Blocker => counts.blocker += 1,
        ApplicationMigrationStatus::Failed => counts.failed += 1,
        ApplicationMigrationStatus::Scanned => counts.scanned += 1,
        ApplicationMigrationStatus::NotScanned => counts.not_scanned += 1,
    }
}

fn aggregate_counts(summary: &mut StatusCounts, part: &StatusCounts) {
    summary.migrated += part.migrated;
    summary.processing += part.processing;
    summary.blocker += part.blocker;
    summary.failed += part.failed;
    summary.scanned += part.scanned;
    summary.not_scanned += part.not_scanned;
}

pub fn aggregate_fleet_summary(parts: &[StatusCounts]) -> StatusCounts {
    let mut summary = StatusCounts::default();
    for part in parts {
        aggregate_counts(&mut summary, part);
    }
    summary.total = summary.migrated
        + summary.processing
        + summary.blocker
        + summary.failed
        + summary.scanned
        + summary.not_scanned;
    summary
}

/// Apply hub Rollout phases onto a dashboard snapshot (DB cache / assessment rebuild).
pub fn overlay_rollout_status(
    response: &mut DashboardResponse,
    rollouts: &HashMap<String, String>,
) {
    if rollouts.is_empty() {
        return;
    }
    for mesh in &mut response.mesh_instances {
        let mut counts = StatusCounts::default();
        for app in &mut mesh.applications {
            if let Some(phase) = rollouts.get(&app.namespace) {
                app.rollout_phase = Some(phase.clone());
                app.status = status_from_rollout_phase(phase, mesh.ambient, app.dataplane_mode.as_str());
            }
            increment_count(&mut counts, app.status);
        }
        counts.total = mesh.applications.len();
        mesh.counts = counts;
    }
    response.summary = StatusCounts::default();
    for mesh in &response.mesh_instances {
        aggregate_counts(&mut response.summary, &mesh.counts);
    }
    response.summary.total = response.summary.migrated
        + response.summary.processing
        + response.summary.blocker
        + response.summary.failed
        + response.summary.scanned
        + response.summary.not_scanned;
}

/// Overlay rollout phases on each fleet cluster entry and re-aggregate fleet summary.
pub fn overlay_fleet_rollout_status(
    fleet: &mut crate::types::FleetDashboardResponse,
    rollouts_by_cluster: &HashMap<String, HashMap<String, String>>,
) {
    let mut summaries = Vec::with_capacity(fleet.clusters.len());
    for cluster in &mut fleet.clusters {
        if let Some(rollouts) = rollouts_by_cluster.get(&cluster.cluster_ref) {
            overlay_rollout_status_parts(
                &mut cluster.mesh_instances,
                &mut cluster.summary,
                rollouts,
            );
        }
        summaries.push(cluster.summary.clone());
    }
    fleet.summary = aggregate_fleet_summary(&summaries);
}

fn overlay_rollout_status_parts(
    mesh_instances: &mut [MeshInstanceDashboard],
    summary: &mut StatusCounts,
    rollouts: &HashMap<String, String>,
) {
    for mesh in &mut *mesh_instances {
        let mut counts = StatusCounts::default();
        for app in &mut mesh.applications {
            if let Some(phase) = rollouts.get(&app.namespace) {
                app.rollout_phase = Some(phase.clone());
                app.status = status_from_rollout_phase(phase, mesh.ambient, app.dataplane_mode.as_str());
            }
            increment_count(&mut counts, app.status);
        }
        counts.total = mesh.applications.len();
        mesh.counts = counts;
    }
    *summary = StatusCounts::default();
    for mesh in mesh_instances {
        aggregate_counts(summary, &mesh.counts);
    }
    summary.total = summary.migrated
        + summary.processing
        + summary.blocker
        + summary.failed
        + summary.scanned
        + summary.not_scanned;
}

fn status_from_rollout_phase(
    phase: &str,
    mesh_ambient: bool,
    dataplane_mode: &str,
) -> ApplicationMigrationStatus {
    match phase {
        "Failed" | "RolledBack" => ApplicationMigrationStatus::Failed,
        "Completed" if mesh_ambient && dataplane_mode == "ambient" => {
            ApplicationMigrationStatus::Migrated
        }
        "Completed" => ApplicationMigrationStatus::Scanned,
        "Running" | "AwaitingApproval" | "Pending" => ApplicationMigrationStatus::Processing,
        _ => ApplicationMigrationStatus::Processing,
    }
}
