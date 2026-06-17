use ambientor_core::rules::{NamespaceContext, RuleContext};
use ambientor_types::MeshFlavor;
use k8s_openapi::api::core::v1::{Namespace, Pod};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{Api, Client};

use crate::istio::collect_istio_policies;
use crate::platform_scan::scan_platform;
use crate::policy_collect::{IstioPolicyObjects, build_policy_context};
use crate::version::detect_istio_version;
use crate::workload_scan::scan_workloads;

/// Optional pre-loaded core resources (e.g. from operator informer cache).
pub type CoreSnapshot = (Vec<Pod>, Vec<Namespace>);

/// Full cluster inventory from one pass (avoids duplicate pod/namespace/Istio lists).
pub struct CollectedInventory {
    pub ctx: RuleContext,
    pub pods: Vec<Pod>,
    pub namespaces: Vec<Namespace>,
    pub istio_objects: IstioPolicyObjects,
}

pub async fn collect_inventory(
    client: &Client,
    flavor: MeshFlavor,
    core: Option<CoreSnapshot>,
) -> anyhow::Result<RuleContext> {
    Ok(collect_inventory_full(client, flavor, core)
        .await?
        .ctx)
}

pub async fn collect_inventory_full(
    client: &Client,
    flavor: MeshFlavor,
    core: Option<CoreSnapshot>,
) -> anyhow::Result<CollectedInventory> {
    let (pods, namespaces) = match core {
        Some((pods, namespaces)) => (pods, namespaces),
        None => fetch_core_resources(client).await?,
    };
    build_collected_inventory(client, flavor, pods, namespaces).await
}

async fn fetch_core_resources(client: &Client) -> anyhow::Result<CoreSnapshot> {
    let ns_api: Api<Namespace> = Api::all(client.clone());
    let pod_api: Api<Pod> = Api::all(client.clone());
    let namespaces = ns_api.list(&Default::default()).await?.items;
    let pods = pod_api.list(&Default::default()).await?.items;
    Ok((pods, namespaces))
}

pub async fn build_rule_context(
    client: &Client,
    flavor: MeshFlavor,
    pods: &[Pod],
    namespaces: &[Namespace],
) -> anyhow::Result<RuleContext> {
    Ok(build_collected_inventory(
        client,
        flavor,
        pods.to_vec(),
        namespaces.to_vec(),
    )
    .await?
    .ctx)
}

async fn build_collected_inventory(
    client: &Client,
    flavor: MeshFlavor,
    pods: Vec<Pod>,
    namespaces: Vec<Namespace>,
) -> anyhow::Result<CollectedInventory> {
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = crd_api.list(&Default::default()).await?;

    let gateway_api = crds.items.iter().any(|c| {
        c.metadata
            .name
            .as_deref()
            .is_some_and(|n| n == "httproutes.gateway.networking.k8s.io")
    });

    let ambient_installed = pods.iter().any(|p| {
        p.metadata.labels.as_ref().is_some_and(|l| {
            l.get("app")
                .is_some_and(|v| v == "ztunnel" || v == "istio-ztunnel")
                || l.get("app.kubernetes.io/name")
                    .is_some_and(|v| v == "ztunnel" || v == "istio-ztunnel")
        })
    });

    let mut ns_contexts = Vec::new();
    for ns in &namespaces {
        let name = ns.metadata.name.clone().unwrap_or_default();
        let labels = ns.metadata.labels.clone().unwrap_or_default();
        let injection = labels
            .get("istio-injection")
            .is_some_and(|v| v == "enabled" || v == "true");
        let ambient = labels
            .get("istio.io/dataplane-mode")
            .is_some_and(|v| v == "ambient");
        let workload_count = pods
            .iter()
            .filter(|p| p.metadata.namespace.as_deref() == Some(name.as_str()))
            .count() as u32;
        let has_vm = pods.iter().any(|p| {
            p.metadata.namespace.as_deref() == Some(name.as_str())
                && p.metadata.labels.as_ref().is_some_and(|l| {
                    l.contains_key("app.kubernetes.io/instance")
                        && l.get("istio.io/workloadInstance").is_some()
                })
        });
        ns_contexts.push(NamespaceContext {
            name,
            injection_enabled: injection,
            ambient_enabled: ambient,
            workload_count,
            has_vm_workloads: has_vm,
        });
    }

    let injected_ns: Vec<String> = ns_contexts
        .iter()
        .filter(|n| n.injection_enabled)
        .map(|n| n.name.clone())
        .collect();
    let workloads = scan_workloads(&pods, &injected_ns);

    let istio_objects = collect_istio_policies(client).await?;
    let policies = build_policy_context(&istio_objects);
    let mesh_version = detect_istio_version(client).await;
    let platform = scan_platform(client, flavor, Some(&pods)).await;

    let ctx = RuleContext {
        mesh_version,
        mesh_flavor: Some(format!("{flavor:?}")),
        ambient_installed,
        gateway_api_present: gateway_api,
        namespaces: ns_contexts,
        workloads,
        policies,
        platform,
    };
    Ok(CollectedInventory {
        ctx,
        pods,
        namespaces,
        istio_objects,
    })
}

pub async fn common_preflight(client: &Client) -> anyhow::Result<Vec<super::PreflightCheck>> {
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());
    let crds = crd_api.list(&Default::default()).await?;
    let gateway = crds.items.iter().any(|c| {
        c.metadata
            .name
            .as_deref()
            .is_some_and(|n| n == "httproutes.gateway.networking.k8s.io")
    });
    Ok(vec![super::PreflightCheck {
        id: "gateway-api-crds".into(),
        passed: gateway,
        message: if gateway {
            "Gateway API CRDs are installed".into()
        } else {
            "Gateway API HTTPRoute CRD is missing".into()
        },
        remediation: Some("Install Gateway API CRDs before ambient migration".into()),
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    fn synthetic_pods(count: usize) -> Vec<Pod> {
        (0..count)
            .map(|i| {
                let ns = format!("ns-{}", i % 50);
                Pod {
                    metadata: ObjectMeta {
                        name: Some(format!("pod-{i}")),
                        namespace: Some(ns),
                        ..Default::default()
                    },
                    spec: Some(k8s_openapi::api::core::v1::PodSpec {
                        containers: vec![k8s_openapi::api::core::v1::Container {
                            name: "istio-proxy".into(),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                }
            })
            .collect()
    }

    fn synthetic_namespaces(count: usize) -> Vec<Namespace> {
        (0..count)
            .map(|i| Namespace {
                metadata: ObjectMeta {
                    name: Some(format!("ns-{i}")),
                    labels: Some(std::collections::BTreeMap::from([(
                        "istio-injection".into(),
                        "enabled".into(),
                    )])),
                    ..Default::default()
                },
                ..Default::default()
            })
            .collect()
    }

    #[test]
    fn core_aggregation_10k_pods_under_budget() {
        let pods = synthetic_pods(10_000);
        let namespaces = synthetic_namespaces(50);
        let injected: Vec<String> = namespaces
            .iter()
            .filter_map(|n| n.metadata.name.clone())
            .collect();

        let start = std::time::Instant::now();
        let mut workload_total = 0u32;
        for ns in &namespaces {
            let name = ns.metadata.name.as_deref().unwrap_or("");
            workload_total += pods
                .iter()
                .filter(|p| p.metadata.namespace.as_deref() == Some(name))
                .count() as u32;
        }
        let workloads = scan_workloads(&pods, &injected);
        let elapsed = start.elapsed();

        assert_eq!(workload_total, 10_000);
        assert!(!workloads.is_empty());
        assert!(
            elapsed.as_millis() < 500,
            "10k pod aggregation took {:?}",
            elapsed
        );
    }
}
