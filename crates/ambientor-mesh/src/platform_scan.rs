use ambientor_core::rules::PlatformContext;
use ambientor_types::MeshFlavor;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{Api, Client};

use crate::dynamic::{api_resource, list_namespaced_cr};
pub async fn scan_platform(
    client: &Client,
    flavor: MeshFlavor,
    pods: Option<&[Pod]>,
) -> PlatformContext {
    let (spire_detected, spire_hits) = detect_spire(client, pods).await;
    let ossm_member_namespaces = if matches!(flavor, MeshFlavor::OSSM3) {
        collect_ossm_member_namespaces(client).await
    } else {
        Vec::new()
    };
    PlatformContext {
        spire_detected,
        spire_hits,
        ossm_member_namespaces,
    }
}

async fn detect_spire(client: &Client, pods: Option<&[Pod]>) -> (bool, Vec<String>) {
    let mut hits = Vec::new();
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    if let Ok(crds) = crd_api.list(&Default::default()).await {
        for c in &crds.items {
            let Some(name) = c.metadata.name.as_deref() else {
                continue;
            };
            if name.contains("spire") || name.contains("spiffeid") || name.contains("spiffe.io") {
                hits.push(format!("crd: {name}"));
            }
        }
    }

    let pod_items: Vec<Pod> = if let Some(slice) = pods {
        slice.to_vec()
    } else {
        let pod_api: Api<Pod> = Api::all(client.clone());
        pod_api
            .list(&Default::default())
            .await
            .map(|list| list.items)
            .unwrap_or_default()
    };

    for pod in &pod_items {
        let Some(name) = pod.metadata.name.as_deref() else {
            continue;
        };
        let ns = pod.metadata.namespace.as_deref().unwrap_or("default");
        if name.contains("spire-agent")
            || name.contains("spire-server")
            || pod.metadata.labels.as_ref().is_some_and(|l| {
                l.iter().any(|(k, v)| {
                    k.contains("spire") || v.contains("spire") || k.contains("spiffe")
                })
            })
        {
            hits.push(format!("pod: {ns}/{name}"));
        }
    }

    (!hits.is_empty(), hits)
}

pub async fn collect_ossm_member_namespaces(client: &Client) -> Vec<String> {
    let ar = api_resource(
        "maistra.io",
        "v1",
        "ServiceMeshMemberRoll",
        "servicemeshmemberrolls",
    );
    let rolls = list_namespaced_cr(client, &ar).await.unwrap_or_default();
    let mut members = Vec::new();
    for roll in rolls {
        if let Some(arr) = roll
            .data
            .get("spec")
            .and_then(|s| s.get("members"))
            .and_then(|m| m.as_array())
        {
            for item in arr {
                if let Some(ns) = item.as_str() {
                    members.push(ns.into());
                }
            }
        }
    }
    members.sort();
    members.dedup();
    members
}
