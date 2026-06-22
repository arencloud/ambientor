use ambientor_core::rules::RuleContext;
use ambientor_dashboard::{
    apply_cluster_ref_metadata, build_cluster_assessment_from_context,
    build_cluster_assessment_from_inventory, cluster_dashboard_meta_with_meshes,
    dashboard_from_assessment_run, merge_mesh_dashboards, mesh_instances_to_dashboard_catalog,
};
use ambientor_k8s::resolve_cluster_display_name;
use ambientor_mesh::inventory::CollectedInventory;
use ambientor_mesh::mesh_instances::discover_mesh_instances;
use ambientor_types::Finding;
use kube::Client;

use crate::pool::DbError;
use crate::traits::{ApplicationAssessmentStore, DashboardStore};

/// Persist application rows and dashboard snapshot from one assessment pass.
pub async fn persist_full_assessment(
    applications: &dyn ApplicationAssessmentStore,
    dashboard: &dyn DashboardStore,
    hub: Option<&Client>,
    spoke: &Client,
    cluster_ref: &str,
    inventory: &CollectedInventory,
    findings: &[Finding],
) -> Result<usize, DbError> {
    let mesh_instances = discover_mesh_instances(spoke)
        .await
        .map_err(|e| DbError::Serialize(e.to_string()))?;

    let run = build_cluster_assessment_from_inventory(
        cluster_ref,
        &inventory.ctx,
        findings,
        &inventory.pods,
        &inventory.namespaces,
        &mesh_instances,
        &inventory.istio_objects,
    );

    let count = run.applications.len();
    applications.replace_run(&run).await?;

    let mut cluster_meta = cluster_dashboard_meta_with_meshes(spoke, &mesh_instances)
        .await
        .map_err(|e| DbError::Serialize(e.to_string()))?;
    cluster_meta.name =
        resolve_cluster_display_name(hub, cluster_ref, &cluster_meta.name).await;

    let mut snapshot = dashboard_from_assessment_run(&run, cluster_meta);
    let catalog = mesh_instances_to_dashboard_catalog(&mesh_instances);
    snapshot.mesh_instances = merge_mesh_dashboards(catalog, snapshot.mesh_instances);
    snapshot.cluster.mesh_instance_count = snapshot.mesh_instances.len();
    snapshot.cluster.ambient_mesh_count = snapshot.mesh_instances.iter().filter(|m| m.ambient).count();
    apply_cluster_ref_metadata(cluster_ref, &mut snapshot);
    dashboard.sync_snapshot(&snapshot).await?;

    Ok(count)
}

/// Legacy path when only `RuleContext` is available (extra API calls).
pub async fn persist_full_assessment_from_context(
    applications: &dyn ApplicationAssessmentStore,
    dashboard: &dyn DashboardStore,
    hub: Option<&Client>,
    spoke: &Client,
    cluster_ref: &str,
    ctx: &RuleContext,
    findings: &[Finding],
) -> Result<usize, DbError> {
    let run = build_cluster_assessment_from_context(spoke, cluster_ref, ctx, findings)
        .await
        .map_err(|e| DbError::Serialize(e.to_string()))?;

    let count = run.applications.len();
    applications.replace_run(&run).await?;

    let mesh_instances = discover_mesh_instances(spoke)
        .await
        .map_err(|e| DbError::Serialize(e.to_string()))?;
    let mut cluster_meta = cluster_dashboard_meta_with_meshes(spoke, &mesh_instances)
        .await
        .map_err(|e| DbError::Serialize(e.to_string()))?;
    cluster_meta.name =
        resolve_cluster_display_name(hub, cluster_ref, &cluster_meta.name).await;

    let mut snapshot = dashboard_from_assessment_run(&run, cluster_meta);
    let catalog = mesh_instances_to_dashboard_catalog(&mesh_instances);
    snapshot.mesh_instances = merge_mesh_dashboards(catalog, snapshot.mesh_instances);
    snapshot.cluster.mesh_instance_count = snapshot.mesh_instances.len();
    snapshot.cluster.ambient_mesh_count = snapshot.mesh_instances.iter().filter(|m| m.ambient).count();
    apply_cluster_ref_metadata(cluster_ref, &mut snapshot);
    dashboard.sync_snapshot(&snapshot).await?;

    Ok(count)
}
