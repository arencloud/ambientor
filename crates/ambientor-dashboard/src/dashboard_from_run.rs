use std::collections::BTreeMap;

use chrono::Utc;

use crate::application_types::{ApplicationAssessmentRecord, ClusterAssessmentRun, RiskLevel};
use crate::dataplane::{DataplaneMode, derive_dataplane_mode_from_stored};
use crate::types::{
    ApplicationMigrationStatus, ApplicationRow, ClusterDashboard, DashboardResponse,
    MeshInstanceDashboard, StatusCounts,
};

/// Build dashboard API payload from the same assessment run stored in `application_assessments`.
pub fn dashboard_from_assessment_run(
    run: &ClusterAssessmentRun,
    cluster: ClusterDashboard,
) -> DashboardResponse {
    let mesh_instances = mesh_groups_from_applications(&run.applications);
    let mut summary = StatusCounts::default();
    for mesh in &mesh_instances {
        aggregate_counts(&mut summary, &mesh.counts);
    }
    summary.total = summary.migrated
        + summary.processing
        + summary.blocker
        + summary.failed
        + summary.scanned
        + summary.not_scanned;

    DashboardResponse {
        cluster_ref: run.cluster_ref.clone(),
        cluster: ClusterDashboard {
            mesh_instance_count: mesh_instances.len(),
            ambient_mesh_count: mesh_instances.iter().filter(|m| m.ambient).count(),
            ..cluster
        },
        summary,
        mesh_instances,
        last_updated: Utc::now().to_rfc3339(),
    }
}

fn mesh_groups_from_applications(apps: &[ApplicationAssessmentRecord]) -> Vec<MeshInstanceDashboard> {
    let mut groups: BTreeMap<(String, String, String), MeshInstanceDashboard> = BTreeMap::new();

    for app in apps {
        let revision = app
            .mesh_revision
            .clone()
            .unwrap_or_else(|| "unknown".into());
        let discovery = app
            .discovery_label
            .clone()
            .unwrap_or_else(|| "unknown".into());
        let cp_ns = app
            .control_plane_namespace
            .clone()
            .unwrap_or_else(|| "unknown".into());
        let key = (revision.clone(), discovery.clone(), cp_ns.clone());

        let mesh = groups.entry(key).or_insert_with(|| MeshInstanceDashboard {
            revision: revision.clone(),
            discovery_label: discovery.clone(),
            control_plane_namespace: cp_ns.clone(),
            version: None,
            ambient: discovery.to_ascii_lowercase().contains("ambient")
                || revision.to_ascii_lowercase().contains("ambient"),
            counts: StatusCounts::default(),
            applications: Vec::new(),
        });

        let status = migration_status_from_app(app);
        let dataplane = resolve_dataplane_mode(app);

        let row = ApplicationRow {
            namespace: app.namespace.clone(),
            status,
            mesh_revision: revision.clone(),
            discovery_label: discovery.clone(),
            dataplane_mode: dataplane.as_str().to_string(),
            ambient_dataplane: dataplane.is_ambient(),
            blocker_count: app.blocker_count as usize,
            rollout_phase: None,
            assessment_ref: Some(format!("assessment/{}", app.namespace)),
        };

        increment_count(&mut mesh.counts, status);
        mesh.applications.push(row);
    }

    let mut meshes: Vec<_> = groups.into_values().collect();
    for mesh in &mut meshes {
        mesh.applications.sort_by(|a, b| a.namespace.cmp(&b.namespace));
        mesh.counts.total = mesh.applications.len();
    }
    meshes.sort_by(|a, b| {
        b.ambient
            .cmp(&a.ambient)
            .then(a.discovery_label.cmp(&b.discovery_label))
    });
    meshes
}

fn resolve_dataplane_mode(app: &ApplicationAssessmentRecord) -> DataplaneMode {
    if !app.dataplane_mode.is_empty() {
        return match app.dataplane_mode.as_str() {
            "ambient" => DataplaneMode::Ambient,
            "sidecar" => DataplaneMode::Sidecar,
            _ => derive_dataplane_mode_from_stored(
                &app.namespace_labels,
                app.mesh_revision.as_deref(),
                app.discovery_label.as_deref(),
            ),
        };
    }
    derive_dataplane_mode_from_stored(
        &app.namespace_labels,
        app.mesh_revision.as_deref(),
        app.discovery_label.as_deref(),
    )
}

fn migration_status_from_app(app: &ApplicationAssessmentRecord) -> ApplicationMigrationStatus {
    if resolve_dataplane_mode(app) == DataplaneMode::Ambient {
        return ApplicationMigrationStatus::Migrated;
    }
    match app.risk_level {
        RiskLevel::Critical | RiskLevel::High if app.blocker_count > 0 => {
            ApplicationMigrationStatus::Blocker
        }
        RiskLevel::Critical => ApplicationMigrationStatus::Failed,
        RiskLevel::High | RiskLevel::Medium => {
            if app.blocker_count > 0 {
                ApplicationMigrationStatus::Blocker
            } else if app.warning_count > 0 || !app.findings.is_empty() {
                ApplicationMigrationStatus::Scanned
            } else {
                ApplicationMigrationStatus::NotScanned
            }
        }
        RiskLevel::Low => {
            if app.findings.is_empty() && app.workload_count == 0 {
                ApplicationMigrationStatus::NotScanned
            } else {
                ApplicationMigrationStatus::Scanned
            }
        }
    }
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
