use ambientor_core::rules::{NamespaceContext, RuleContext};
use ambientor_types::MeshFlavor;
use k8s_openapi::api::core::v1::{Namespace, Pod};
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{Api, Client};

use crate::istio::collect_istio_policies;
use crate::policy_collect::build_policy_context;
use crate::version::detect_istio_version;
use crate::workload_scan::scan_workloads;

pub async fn collect_inventory(client: &Client, flavor: MeshFlavor) -> anyhow::Result<RuleContext> {
    let ns_api: Api<Namespace> = Api::all(client.clone());
    let pod_api: Api<Pod> = Api::all(client.clone());
    let crd_api: Api<CustomResourceDefinition> = Api::all(client.clone());

    let namespaces = ns_api.list(&Default::default()).await?;
    let pods = pod_api.list(&Default::default()).await?;
    let crds = crd_api.list(&Default::default()).await?;

    let gateway_api = crds.items.iter().any(|c| {
        c.metadata
            .name
            .as_deref()
            .is_some_and(|n| n == "httproutes.gateway.networking.k8s.io")
    });

    let ambient_installed = pods.items.iter().any(|p| {
        p.metadata
            .labels
            .as_ref()
            .is_some_and(|l| l.get("app").is_some_and(|v| v == "ztunnel"))
    });

    let mut ns_contexts = Vec::new();
    for ns in &namespaces.items {
        let name = ns.metadata.name.clone().unwrap_or_default();
        let labels = ns.metadata.labels.clone().unwrap_or_default();
        let injection = labels
            .get("istio-injection")
            .is_some_and(|v| v == "enabled" || v == "true");
        let ambient = labels
            .get("istio.io/dataplane-mode")
            .is_some_and(|v| v == "ambient");
        let workload_count = pods
            .items
            .iter()
            .filter(|p| p.metadata.namespace.as_deref() == Some(name.as_str()))
            .count() as u32;
        let has_vm = pods.items.iter().any(|p| {
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
    let workloads = scan_workloads(&pods.items, &injected_ns);

    let istio_objects = collect_istio_policies(client).await?;
    let policies = build_policy_context(&istio_objects);
    let mesh_version = detect_istio_version(client).await;

    Ok(RuleContext {
        mesh_version,
        mesh_flavor: Some(format!("{flavor:?}")),
        ambient_installed,
        gateway_api_present: gateway_api,
        namespaces: ns_contexts,
        workloads,
        policies,
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
