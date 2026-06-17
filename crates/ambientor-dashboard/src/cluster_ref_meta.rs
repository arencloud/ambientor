use crate::DashboardResponse;

/// Attach hub/spoke metadata used by Postgres registry and the Web UI.
pub fn apply_cluster_ref_metadata(cluster_ref: &str, response: &mut DashboardResponse) {
    response.cluster_ref = cluster_ref.to_string();
    if let Some((ns, name)) = ambientor_k8s::parse_connection_cluster_ref(cluster_ref) {
        response.connection_namespace = Some(ns.to_string());
        response.connection_name = Some(name.to_string());
        response.is_hub = Some(false);
        response.reachable = Some(true);
    } else {
        response.connection_namespace = None;
        response.connection_name = None;
        response.is_hub = Some(true);
        response.reachable = Some(true);
    }
}
