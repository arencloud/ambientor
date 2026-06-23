use ambientor_core::rules::RuleContext;
use ambientor_k8s::detect_platform;
use ambientor_mesh::mesh_instances::discover_mesh_instances;
use ambientor_mesh::policy_collect::IstioPolicyObjects;
use ambientor_mesh::version::detect_istio_version;
use ambientor_types::{Finding, MeshInstance};
use k8s_openapi::api::core::v1::{Namespace, Pod};
use kube::Client;

use crate::application_types::ClusterAssessmentRun;
use crate::applications::{
    build_cluster_assessment, hostnames_from_istio_objects, ingress_gateway_namespaces_from_pods,
};
use crate::compute::load_cluster_display_name;
use crate::types::ClusterDashboard;
use ambientor_mesh::application_identity::identities_by_namespace;

pub use crate::dashboard_from_run::dashboard_from_assessment_run;

/// Build application assessment rows reusing pods/namespaces/Istio objects from the scan pass.
pub fn build_cluster_assessment_from_inventory(
    cluster_ref: &str,
    ctx: &RuleContext,
    findings: &[Finding],
    pods: &[Pod],
    namespaces: &[Namespace],
    mesh_instances: &[MeshInstance],
    istio_objects: &IstioPolicyObjects,
) -> ClusterAssessmentRun {
    let hostnames = hostnames_from_istio_objects(istio_objects);
    let ingress = ingress_gateway_namespaces_from_pods(pods);
    let identities = identities_by_namespace(pods);
    build_cluster_assessment(
        cluster_ref,
        ctx,
        findings,
        namespaces,
        mesh_instances,
        &hostnames,
        &ingress,
        &identities,
    )
}

pub async fn build_cluster_assessment_from_context(
    client: &Client,
    cluster_ref: &str,
    ctx: &RuleContext,
    findings: &[Finding],
) -> anyhow::Result<ClusterAssessmentRun> {
    use crate::applications::{hostnames_by_namespace, list_namespaces_for_assessment};
    use kube::Api;
    use kube::api::ListParams;

    let namespaces = list_namespaces_for_assessment(client).await?;
    let mesh_instances = discover_mesh_instances(client).await?;
    let hostnames = hostnames_by_namespace(client).await?;
    let pod_api: Api<Pod> = Api::all(client.clone());
    let pods = pod_api.list(&ListParams::default()).await?.items;
    let ingress = ingress_gateway_namespaces_from_pods(&pods);
    let identities = identities_by_namespace(&pods);
    Ok(build_cluster_assessment(
        cluster_ref,
        ctx,
        findings,
        &namespaces,
        &mesh_instances,
        &hostnames,
        &ingress,
        &identities,
    ))
}

pub async fn cluster_dashboard_meta(client: &Client) -> anyhow::Result<ClusterDashboard> {
    let mesh_instances = discover_mesh_instances(client).await?;
    cluster_dashboard_meta_with_meshes(client, &mesh_instances).await
}

pub async fn cluster_dashboard_meta_with_meshes(
    client: &Client,
    mesh_instances: &[MeshInstance],
) -> anyhow::Result<ClusterDashboard> {
    let platform = detect_platform(client).await?;
    let istio_version = detect_istio_version(client).await;
    let name = load_cluster_display_name(client).await;
    Ok(ClusterDashboard {
        name,
        platform: if platform.is_openshift {
            "OpenShift".into()
        } else {
            "Kubernetes".into()
        },
        mesh_flavor: format!("{:?}", platform.mesh_flavor),
        istio_version,
        mesh_instance_count: mesh_instances.len(),
        ambient_mesh_count: mesh_instances.iter().filter(|m| m.ambient).count(),
    })
}
