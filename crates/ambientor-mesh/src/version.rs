use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::api::core::v1::Namespace;
use kube::api::ListParams;
use kube::{Api, Client};

/// Legacy single-namespace install; Sail/OSSM uses `<revision>-istio-system`.
const LEGACY_ISTIOD_NAMESPACE: &str = "istio-system";

/// Best-effort Istio control-plane version from istiod deployments (image tag, revision label, or name).
pub async fn detect_istio_version(client: &Client) -> Option<String> {
    for ns in istiod_search_namespaces(client).await {
        if let Some(v) = detect_istio_version_in_namespace(client, &ns).await {
            return Some(v);
        }
    }
    detect_istio_version_cluster_wide(client).await
}

async fn istiod_search_namespaces(client: &Client) -> Vec<String> {
    let mut namespaces = vec![LEGACY_ISTIOD_NAMESPACE.to_string()];
    let api: Api<Namespace> = Api::all(client.clone());
    if let Ok(list) = api.list(&ListParams::default()).await {
        for ns in list.items {
            if let Some(name) = ns.metadata.name
                && name.ends_with("-istio-system")
                && !namespaces.iter().any(|n| n == &name)
            {
                namespaces.push(name);
            }
        }
    }
    namespaces
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
    let mut best: Option<(u32, u32, u32, String)> = None;
    for dep in deployments {
        let name = dep.metadata.name.as_deref().unwrap_or("");
        if !name.contains("istiod") && !deployment_has_istiod_label(dep) {
            continue;
        }
        if let Some(v) = version_from_deployment(dep, name)
            && let Some((maj, min, patch)) = parse_semver_triple(&v)
        {
            let replaces = best.as_ref().is_none_or(|(bmaj, bmin, bpatch, _)| {
                (maj, min, patch) > (*bmaj, *bmin, *bpatch)
            });
            if replaces {
                best = Some((maj, min, patch, v));
            }
        }
    }
    best.map(|(_, _, _, v)| v)
}

fn deployment_has_istiod_label(dep: &Deployment) -> bool {
    dep.metadata
        .labels
        .as_ref()
        .is_some_and(|l| l.get("app").is_some_and(|a| a == "istiod"))
}

fn version_from_deployment(dep: &Deployment, deployment_name: &str) -> Option<String> {
    for rev in istio_revision_labels(dep) {
        if let Some(v) = parse_revision_version(&rev) {
            return Some(v);
        }
        if looks_like_istio_version(&rev) {
            return Some(rev);
        }
    }
    if let Some(v) = parse_revision_version(deployment_name) {
        return Some(v);
    }
    dep.spec.as_ref().and_then(|s| {
        s.template
            .spec
            .as_ref()
            .and_then(|ps| ps.containers.first())
            .and_then(|c| c.image.as_ref())
            .and_then(|img| img.rsplit_once(':').map(|(_, tag)| tag))
            .and_then(|tag| {
                if looks_like_istio_version(tag) {
                    Some(tag.to_string())
                } else {
                    parse_revision_version(tag)
                }
            })
    })
}

fn istio_revision_labels(dep: &Deployment) -> Vec<String> {
    let mut revs = Vec::new();
    if let Some(rev) = dep
        .metadata
        .labels
        .as_ref()
        .and_then(|l| l.get("istio.io/rev").cloned())
    {
        revs.push(rev);
    }
    if let Some(rev) = dep
        .spec
        .as_ref()
        .and_then(|s| s.template.metadata.as_ref())
        .and_then(|m| m.labels.as_ref())
        .and_then(|l| l.get("istio.io/rev").cloned())
    {
        if !revs.contains(&rev) {
            revs.push(rev);
        }
    }
    revs
}

/// Parse Sail/OSSM revision strings such as `ambient-v1-28-6` or deployment names `istiod-ambient-v1-28-6`.
pub fn parse_revision_version(value: &str) -> Option<String> {
    let lower = value.to_ascii_lowercase();
    let v_idx = lower.rfind('v')?;
    let tail = &value[v_idx + 1..];
    let parts: Vec<&str> = tail.split('-').collect();
    if parts.len() < 2 {
        return None;
    }
    let major: u32 = parts[0].parse().ok()?;
    let minor: u32 = parts[1].parse().ok()?;
    let patch: u32 = parts
        .get(2)
        .and_then(|p| p.parse().ok())
        .unwrap_or(0);
    Some(format!("{major}.{minor}.{patch}"))
}

fn parse_semver_triple(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.trim().split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    let patch: u32 = parts.next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

/// True for `1.24.2`-style tags; revision names like `default` are not versions.
fn looks_like_istio_version(value: &str) -> bool {
    parse_semver_triple(value).is_some_and(|(major, minor, _)| (major, minor) != (0, 0))
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

    #[test]
    fn parses_sail_revision_labels() {
        assert_eq!(
            parse_revision_version("ambient-v1-28-6").as_deref(),
            Some("1.28.6")
        );
        assert_eq!(
            parse_revision_version("demo-v1-28-6").as_deref(),
            Some("1.28.6")
        );
        assert_eq!(
            parse_revision_version("istiod-ambient-v1-28-6").as_deref(),
            Some("1.28.6")
        );
    }
}
