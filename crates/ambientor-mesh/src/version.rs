use k8s_openapi::api::apps::v1::Deployment;
use kube::api::ListParams;
use kube::{Api, Client};

/// Namespaces where OSSM / upstream Istio commonly runs istiod.
const ISTIOD_NAMESPACES: &[&str] = &["istio-system"];

/// Best-effort Istio control-plane version from istiod deployment image tag or `istio.io/rev` label.
pub async fn detect_istio_version(client: &Client) -> Option<String> {
    for ns in ISTIOD_NAMESPACES {
        if let Some(v) = detect_istio_version_in_namespace(client, ns).await {
            return Some(v);
        }
    }
    detect_istio_version_cluster_wide(client).await
}

async fn detect_istio_version_in_namespace(client: &Client, namespace: &str) -> Option<String> {
    let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    let deployments = api.list(&Default::default()).await.ok()?;
    version_from_deployments(&deployments.items)
}

async fn detect_istio_version_cluster_wide(client: &Client) -> Option<String> {
    let api: Api<Deployment> = Api::all(client.clone());
    let deployments = api
        .list(&ListParams::default().labels("app=istiod"))
        .await
        .ok()?;
    version_from_deployments(&deployments.items)
}

fn version_from_deployments(deployments: &[Deployment]) -> Option<String> {
    let mut best: Option<(u32, u32, String)> = None;
    for dep in deployments {
        let name = dep.metadata.name.as_deref().unwrap_or("");
        if !name.contains("istiod") && !deployment_has_istiod_label(dep) {
            continue;
        }
        if let Some(v) = version_from_deployment(dep) {
            if let Some((maj, min)) = parse_major_minor(&v) {
                let replaces = best
                    .as_ref()
                    .is_none_or(|(bmaj, bmin, _)| (maj, min) > (*bmaj, *bmin));
                if replaces {
                    best = Some((maj, min, v));
                }
            }
        }
    }
    best.map(|(_, _, v)| v)
}

fn deployment_has_istiod_label(dep: &Deployment) -> bool {
    dep.metadata
        .labels
        .as_ref()
        .is_some_and(|l| l.get("app").is_some_and(|a| a == "istiod"))
}

fn version_from_deployment(dep: &Deployment) -> Option<String> {
    if let Some(rev) = dep
        .metadata
        .labels
        .as_ref()
        .and_then(|l| l.get("istio.io/rev"))
        .filter(|rev| looks_like_istio_version(rev))
    {
        return Some(rev.clone());
    }
    if let Some(rev) = dep
        .spec
        .as_ref()
        .and_then(|s| s.template.metadata.as_ref())
        .and_then(|m| m.labels.as_ref())
        .and_then(|l| l.get("istio.io/rev"))
        .filter(|rev| looks_like_istio_version(rev))
    {
        return Some(rev.clone());
    }
    dep.spec.as_ref().and_then(|s| {
        s.template
            .spec
            .as_ref()
            .and_then(|ps| ps.containers.first())
            .and_then(|c| c.image.as_ref())
            .and_then(|img| img.rsplit_once(':').map(|(_, tag)| tag))
            .filter(|tag| looks_like_istio_version(tag))
            .map(|tag| tag.to_string())
    })
}

fn parse_major_minor(version: &str) -> Option<(u32, u32)> {
    let mut parts = version.trim().split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    Some((major, minor))
}

/// True for `1.24.2`-style tags; revision names like `default` are not versions.
fn looks_like_istio_version(value: &str) -> bool {
    parse_major_minor(value).is_some_and(|(major, minor)| (major, minor) != (0, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_default_is_not_a_version() {
        assert!(!looks_like_istio_version("default"));
    }

    #[test]
    fn image_tag_and_rev_are_versions() {
        assert!(looks_like_istio_version("1.24.2"));
        assert!(looks_like_istio_version("1.28.6"));
    }

    #[test]
    fn ossm_placeholder_is_not_a_version() {
        assert!(!looks_like_istio_version("ossm-3"));
    }
}
