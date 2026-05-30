//! Resolve Istio `RevisionTag` / `IstioRevisionTag` names for namespace `istio.io/rev` labels.

use std::collections::HashMap;

use kube::Client;

use crate::dynamic::{api_resource, list_cluster_cr, list_cr_in_namespace};

const REVISION_TAG_APIS: &[(&str, &str, &str, &str)] = &[
    ("tags.istio.io", "v1alpha3", "RevisionTag", "revisiontags"),
    ("tags.istio.io", "v1alpha1", "RevisionTag", "revisiontags"),
    ("istio.io", "v1alpha1", "IstioRevisionTag", "istiorevisiontags"),
];

/// Map istiod deployment revision → preferred `istio.io/rev` label value (revision tag when set).
pub async fn revision_tags_by_istiod_revision(client: &Client) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for (group, version, kind, plural) in REVISION_TAG_APIS {
        let ar = api_resource(group, version, kind, plural);
        let tags = list_cluster_cr(client, &ar).await.unwrap_or_default();
        for tag in tags {
            merge_revision_tag(&mut out, &tag);
        }
        let tags_ns = list_namespaced_cr_all(client, &ar).await;
        for tag in tags_ns {
            merge_revision_tag(&mut out, &tag);
        }
    }
    out
}

/// Preferred namespace label value for `istio.io/rev` on this control plane.
pub async fn preferred_namespace_revision_label(
    client: &Client,
    control_plane_namespace: &str,
    istiod_revision: &str,
) -> (String, Option<String>) {
    let map = revision_tags_by_istiod_revision(client).await;
    if let Some(tag) = map.get(istiod_revision) {
        return (tag.clone(), Some(tag.clone()));
    }
    if let Some(tag) = tags_in_namespace(client, control_plane_namespace, istiod_revision).await {
        return (tag.clone(), Some(tag));
    }
    (istiod_revision.to_string(), None)
}

async fn tags_in_namespace(
    client: &Client,
    namespace: &str,
    istiod_revision: &str,
) -> Option<String> {
    for (group, version, kind, plural) in REVISION_TAG_APIS {
        let ar = api_resource(group, version, kind, plural);
        let tags = list_cr_in_namespace(client, &ar, namespace)
            .await
            .unwrap_or_default();
        for tag in tags {
            if let Some((rev, tag_name)) = parse_revision_tag(&tag)
                && rev == istiod_revision
            {
                return Some(tag_name);
            }
        }
    }
    None
}

async fn list_namespaced_cr_all(
    client: &Client,
    ar: &kube::api::ApiResource,
) -> Vec<kube::api::DynamicObject> {
    use k8s_openapi::api::core::v1::Namespace;
    use kube::api::ListParams;
    use kube::Api;

    let ns_api: Api<Namespace> = Api::all(client.clone());
    let Ok(list) = ns_api.list(&ListParams::default()).await else {
        return Vec::new();
    };
    let mut all = Vec::new();
    for ns in list.items {
        let Some(name) = ns.metadata.name else {
            continue;
        };
        if let Ok(items) = list_cr_in_namespace(client, ar, &name).await {
            all.extend(items);
        }
    }
    all
}

fn merge_revision_tag(out: &mut HashMap<String, String>, tag: &kube::api::DynamicObject) {
    if let Some((istiod_rev, tag_name)) = parse_revision_tag(tag) {
        // Prefer first tag seen; do not overwrite with duplicate mappings.
        out.entry(istiod_rev).or_insert(tag_name);
    }
}

fn parse_revision_tag(tag: &kube::api::DynamicObject) -> Option<(String, String)> {
    let tag_name = tag.metadata.name.clone()?;
    let spec = tag.data.get("spec")?;
    let target_revision = spec
        .get("revision")
        .or_else(|| spec.get("targetRevision"))
        .and_then(|v| v.as_str())
        .map(str::to_string)?;
    Some((target_revision, tag_name))
}

#[cfg(test)]
mod tests {
    use kube::api::DynamicObject;
    use kube::api::ObjectMeta;
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_revision_tag_spec() {
        let parsed = parse_revision_tag(&DynamicObject {
            metadata: ObjectMeta {
                name: Some("prod-stable".into()),
                namespace: Some("istio-system".into()),
                ..Default::default()
            },
            data: json!({ "spec": { "revision": "ambient-v1-28-6" } }),
            types: None,
        });
        assert_eq!(
            parsed,
            Some(("ambient-v1-28-6".into(), "prod-stable".into()))
        );
    }
}
