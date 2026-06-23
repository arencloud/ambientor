use ambientor_types::ClusterConnection;
use kube::{Api, Client};

use crate::remote::{connection_cluster_ref, parse_connection_cluster_ref};

/// Human-readable cluster title for UI and Postgres `clusters.display_name`.
pub async fn resolve_cluster_display_name(
    hub: Option<&Client>,
    cluster_ref: &str,
    live_name: &str,
) -> String {
    if let (Some(hub), Some((ns, name))) = (hub, parse_connection_cluster_ref(cluster_ref))
        && let Ok(conn) = fetch_connection(hub, ns, name).await
    {
        return conn.spec.display_name;
    }
    if cluster_ref == "in-cluster" {
        if live_name != "In-cluster" && live_name != "Connected cluster" {
            return live_name.to_string();
        }
        return std::env::var("CLUSTER_NAME")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Hub cluster".into());
    }
    if live_name != "In-cluster" && live_name != "Connected cluster" {
        return live_name.to_string();
    }
    cluster_ref.to_string()
}

pub async fn fetch_connection(
    hub: &Client,
    namespace: &str,
    name: &str,
) -> Result<ClusterConnection, kube::Error> {
    let api: Api<ClusterConnection> = Api::namespaced(hub.clone(), namespace);
    api.get(name).await
}

/// Map `cluster_ref` → `ClusterConnection.spec.display_name` for hub and spokes.
pub async fn connection_display_names(
    hub: &Client,
    hub_cluster_ref: &str,
) -> Result<std::collections::HashMap<String, String>, kube::Error> {
    let api: Api<ClusterConnection> = Api::all(hub.clone());
    let list = api.list(&kube::api::ListParams::default()).await?;
    let mut map = std::collections::HashMap::new();
    for conn in list.items {
        let Some(name) = conn.metadata.name else {
            continue;
        };
        let ns = conn.metadata.namespace.unwrap_or_else(|| "default".into());
        let cluster_ref = if conn.spec.hub {
            hub_cluster_ref.to_string()
        } else {
            connection_cluster_ref(&ns, &name)
        };
        map.insert(cluster_ref, conn.spec.display_name);
    }
    Ok(map)
}
