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
        if let Some(tag) = dep.spec.as_ref().and_then(|s| {
            s.template
                .spec
                .as_ref()
                .and_then(|ps| ps.containers.first())
                .and_then(|c| c.image.as_ref())
                .and_then(|img| img.rsplit_once(':').map(|(_, tag)| tag))
        }) && looks_like_istio_version(tag)
        {
            return Some(tag.to_string());
        }
        if let Some(rev) = dep
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("istio.io/rev"))
            && looks_like_istio_version(rev)
        {
            return Some(rev.to_string());
        }
    }
    None
}

/// True for `1.24.2`-style tags; revision names like `default` are not versions.
fn looks_like_istio_version(value: &str) -> bool {
    let mut parts = value.trim().split('.');
    let Some(major) = parts.next().and_then(|p| p.parse::<u32>().ok()) else {
        return false;
    };
    let Some(minor) = parts.next().and_then(|p| p.parse::<u32>().ok()) else {
        return false;
    };
    (major, minor) != (0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_default_is_not_a_version() {
        assert!(!looks_like_istio_version("default"));
    }

    #[test]
    fn image_tag_is_a_version() {
        assert!(looks_like_istio_version("1.24.2"));
    }
}
