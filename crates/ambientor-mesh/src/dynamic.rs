use kube::api::{ApiResource, DynamicObject};
use kube::{Api, Client};

/// List cluster-scoped custom resources by API resource descriptor.
pub async fn list_cluster_cr(
    client: &Client,
    ar: &ApiResource,
) -> anyhow::Result<Vec<DynamicObject>> {
    let api = Api::<DynamicObject>::all_with(client.clone(), ar);
    Ok(api.list(&Default::default()).await?.items)
}

/// List namespaced custom resources across all namespaces.
pub async fn list_namespaced_cr(
    client: &Client,
    ar: &ApiResource,
) -> anyhow::Result<Vec<DynamicObject>> {
    let api = Api::<DynamicObject>::all_with(client.clone(), ar);
    Ok(api.list(&Default::default()).await?.items)
}

pub fn api_resource(group: &str, version: &str, kind: &str, plural: &str) -> ApiResource {
    ApiResource {
        group: group.into(),
        version: version.into(),
        kind: kind.into(),
        api_version: format!("{group}/{version}"),
        plural: plural.into(),
    }
}

pub fn resource_ref(obj: &DynamicObject) -> String {
    let ns = obj
        .metadata
        .namespace
        .as_deref()
        .map(|n| format!("{n}/"))
        .unwrap_or_default();
    let name = obj.metadata.name.as_deref().unwrap_or("unknown");
    format!("{ns}{name}")
}
