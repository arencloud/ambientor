use k8s_openapi::api::apps::v1::Deployment;
use kube::{Api, Client};

/// Best-effort Istio control-plane version from istiod deployment labels.
pub async fn detect_istio_version(client: &Client) -> Option<String> {
    let api: Api<Deployment> = Api::namespaced(client.clone(), "istio-system");
    let deployments = api.list(&Default::default()).await.ok()?;
    for dep in deployments.items {
        let name = dep.metadata.name.as_deref().unwrap_or("");
        if !name.contains("istiod") {
            continue;
        }
        if let Some(version) = dep
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("istio.io/rev"))
            .map(|s| s.to_string())
        {
            return Some(version);
        }
        if let Some(tag) = dep.spec.as_ref().and_then(|s| {
            s.template
                .spec
                .as_ref()
                .and_then(|ps| ps.containers.first())
                .and_then(|c| c.image.as_ref())
                .and_then(|img| img.rsplit_once(':').map(|(_, tag)| tag))
        }) {
            return Some(tag.to_string());
        }
    }
    None
}
