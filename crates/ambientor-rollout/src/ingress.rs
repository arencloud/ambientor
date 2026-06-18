use std::collections::BTreeMap;
use std::time::Duration;

use ambientor_mesh::dynamic::{api_resource, list_cr_in_namespace};
use ambientor_mesh::ingress_collect::{
    build_ingress_context, gateway_for_route,
};
use ambientor_types::{AmbientIngressGateway, MeshInstance};
use kube::api::{DeleteParams, DynamicObject, Patch, PatchParams};
use kube::{Api, Client};
use serde_json::json;
use tokio::time::sleep;
use tracing::info;

use crate::apply::apply_namespaced_manifest;
use crate::engine::RolloutError;
use crate::verify::gateway_ready;

pub const PER_NAMESPACE_INGRESS_NAME: &str = "ambient-ingress";
const MANAGED_BY_LABEL: &str = "app.kubernetes.io/managed-by";
const MANAGED_BY_VALUE: &str = "ambientor";
const ORIGINAL_PARENT_REFS_ANNOTATION: &str = "ambientor.io/original-parent-refs";
const ORIGINAL_ROUTE_TARGET_ANNOTATION: &str = "ambientor.io/original-route-target";
const GATEWAY_READY_TIMEOUT_SECS: u64 = 180;
const GATEWAY_POLL_INTERVAL_SECS: u64 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResolvedIngressGateway {
    namespace: String,
    name: String,
    /// Ambientor created this Gateway during migration (safe to delete on rollback).
    created: bool,
}

/// Ensure an ambient ingress Gateway exists and repoint app HTTPRoutes (and OpenShift Routes).
pub async fn migrate_ambient_ingress(
    client: &Client,
    namespace: &str,
    mesh: &MeshInstance,
    shared: Option<&AmbientIngressGateway>,
) -> Result<String, RolloutError> {
    let (routes, ingress_gateways) = collect_namespace_routes(client, namespace).await?;
    let public_routes: Vec<_> = routes
        .iter()
        .filter(|r| !r.hostnames.is_empty() || r.parent_gateway_name.is_some())
        .collect();
    if public_routes.is_empty() {
        return Ok(format!(
            "no external HTTPRoutes in {namespace}; skipped ingress migration"
        ));
    }

    let target = resolve_ingress_gateway(client, namespace, mesh, shared).await?;
    let mut migrated_routes = 0usize;
    let mut legacy_services = Vec::new();
    let mut route_errors = Vec::new();

    for route in &public_routes {
        if route_already_on_target(route, &target, &ingress_gateways) {
            continue;
        }
        if let Some(gw) = gateway_for_route(route, &ingress_gateways) {
            legacy_services.push(istio_gateway_service_name(&gw.name));
        } else if let Some(gname) = route.parent_gateway_name.as_ref() {
            legacy_services.push(istio_gateway_service_name(gname));
        }
        if let Err(e) = patch_httproute_parent_refs(
            client,
            namespace,
            &route.name,
            &target.namespace,
            &target.name,
        )
        .await
        {
            route_errors.push(e);
            continue;
        }
        migrated_routes += 1;
    }

    let new_service = istio_gateway_service_name(&target.name);
    let routes_updated =
        migrate_openshift_routes(client, &legacy_services, &target.namespace, &new_service).await?;

    if !route_errors.is_empty() {
        let _ = revert_ambient_ingress(client, namespace, shared).await;
        return Err(route_errors
            .into_iter()
            .next()
            .unwrap_or_else(|| RolloutError::ExecutionFailed("HTTPRoute migration failed".into())));
    }

    Ok(format!(
        "ambient ingress {}/{} (created={}); migrated {migrated_routes} HTTPRoute(s); updated {routes_updated} OpenShift Route(s)",
        target.namespace, target.name, target.created
    ))
}

pub async fn revert_ambient_ingress(
    client: &Client,
    namespace: &str,
    shared: Option<&AmbientIngressGateway>,
) -> Result<String, RolloutError> {
    let mut notes = Vec::new();
    let restored = restore_httproute_parent_refs(client, namespace).await?;
    if restored > 0 {
        notes.push(format!("restored parentRefs on {restored} HTTPRoute(s)"));
    }
    let routes_reverted = revert_openshift_routes(client).await?;
    if routes_reverted > 0 {
        notes.push(format!("reverted {routes_reverted} OpenShift Route(s)"));
    }
    if shared.is_none() {
        if delete_managed_gateway(client, namespace, PER_NAMESPACE_INGRESS_NAME).await? {
            notes.push(format!(
                "deleted per-namespace ingress Gateway {namespace}/{PER_NAMESPACE_INGRESS_NAME}"
            ));
        }
    } else if let Some(shared) = shared {
        if delete_managed_gateway(client, &shared.namespace, &shared.name).await? {
            notes.push(format!(
                "deleted shared ingress Gateway {}/{}",
                shared.namespace, shared.name
            ));
        }
    }
    if notes.is_empty() {
        Ok("no ingress migration resources to revert".into())
    } else {
        Ok(notes.join("; "))
    }
}

async fn resolve_ingress_gateway(
    client: &Client,
    app_namespace: &str,
    mesh: &MeshInstance,
    shared: Option<&AmbientIngressGateway>,
) -> Result<ResolvedIngressGateway, RolloutError> {
    if let Some(shared) = shared {
        let existing = get_gateway(client, &shared.namespace, &shared.name).await?;
        if let Some(gw) = existing {
            if !gateway_ready(&gw.data) {
                wait_gateway_programmed(client, &shared.namespace, &shared.name).await?;
            }
            return Ok(ResolvedIngressGateway {
                namespace: shared.namespace.clone(),
                name: shared.name.clone(),
                created: false,
            });
        }
        deploy_ingress_gateway(client, &shared.namespace, &shared.name, mesh).await?;
        wait_gateway_programmed(client, &shared.namespace, &shared.name).await?;
        return Ok(ResolvedIngressGateway {
            namespace: shared.namespace.clone(),
            name: shared.name.clone(),
            created: true,
        });
    }

    let existing = get_gateway(client, app_namespace, PER_NAMESPACE_INGRESS_NAME).await?;
    if let Some(gw) = existing {
        if gateway_ready(&gw.data) {
            return Ok(ResolvedIngressGateway {
                namespace: app_namespace.to_string(),
                name: PER_NAMESPACE_INGRESS_NAME.into(),
                created: false,
            });
        }
        wait_gateway_programmed(client, app_namespace, PER_NAMESPACE_INGRESS_NAME).await?;
        return Ok(ResolvedIngressGateway {
            namespace: app_namespace.to_string(),
            name: PER_NAMESPACE_INGRESS_NAME.into(),
            created: false,
        });
    }

    deploy_ingress_gateway(client, app_namespace, PER_NAMESPACE_INGRESS_NAME, mesh).await?;
    wait_gateway_programmed(client, app_namespace, PER_NAMESPACE_INGRESS_NAME).await?;
    Ok(ResolvedIngressGateway {
        namespace: app_namespace.to_string(),
        name: PER_NAMESPACE_INGRESS_NAME.into(),
        created: true,
    })
}

async fn deploy_ingress_gateway(
    client: &Client,
    namespace: &str,
    name: &str,
    mesh: &MeshInstance,
) -> Result<(), RolloutError> {
    let labels = ambient_gateway_labels(mesh);
    let manifest = json!({
        "apiVersion": "gateway.networking.k8s.io/v1",
        "kind": "Gateway",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": labels,
        },
        "spec": {
            "gatewayClassName": "istio",
            "listeners": [{
                "name": "http",
                "port": 8080,
                "protocol": "HTTP",
                "allowedRoutes": {
                    "namespaces": { "from": "All" }
                }
            }]
        }
    });
    apply_namespaced_manifest(client, namespace, &manifest).await?;
    info!(namespace = %namespace, gateway = %name, "deployed ambient ingress Gateway");
    Ok(())
}

fn ambient_gateway_labels(mesh: &MeshInstance) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    labels.insert(MANAGED_BY_LABEL.into(), MANAGED_BY_VALUE.into());
    labels.insert("ambientor.io/ingress-role".into(), "ambient".into());
    labels.insert("ambientor.io/ingress-created".into(), "true".into());
    let rev = mesh
        .enrollment
        .revision_tag
        .as_deref()
        .unwrap_or(mesh.enrollment.revision.as_str());
    labels.insert("istio.io/rev".into(), rev.to_string());
    if let Some(value) = mesh.enrollment.discovery_label_value.as_ref() {
        let key = mesh
            .enrollment
            .discovery_label_key
            .as_deref()
            .unwrap_or("istio-discovery");
        labels.insert(key.to_string(), value.clone());
    }
    labels
}

fn istio_gateway_service_name(gateway_name: &str) -> String {
    format!("{gateway_name}-istio")
}

fn route_already_on_target(
    route: &ambientor_core::rules::ExternalRouteInfo,
    target: &ResolvedIngressGateway,
    _gateways: &[ambientor_core::rules::IngressGatewayInfo],
) -> bool {
    route.parent_gateway_namespace.as_deref() == Some(target.namespace.as_str())
        && route.parent_gateway_name.as_deref() == Some(target.name.as_str())
        && route.parents_attached != Some(false)
}

async fn collect_namespace_routes(
    client: &Client,
    namespace: &str,
) -> Result<
    (
        Vec<ambientor_core::rules::ExternalRouteInfo>,
        Vec<ambientor_core::rules::IngressGatewayInfo>,
    ),
    RolloutError,
> {
    let hr_ar = api_resource("gateway.networking.k8s.io", "v1", "HTTPRoute", "httproutes");
    let vs_ar = api_resource(
        "networking.istio.io",
        "v1",
        "VirtualService",
        "virtualservices",
    );
    let routes = list_cr_in_namespace(client, &hr_ar, namespace)
        .await
        .map_err(|e| RolloutError::ExecutionFailed(e.to_string()))?;
    let virtual_services = list_cr_in_namespace(client, &vs_ar, namespace)
        .await
        .unwrap_or_default();
    let gateways = cluster_ingress_gateways(client).await?;
    let (_, external_routes) =
        build_ingress_context(&gateways, &[], &routes, &virtual_services);
    let ingress_gateways: Vec<_> = build_ingress_context(&gateways, &[], &[], &[])
        .0
        .into_iter()
        .filter(|g| g.gateway_class.as_deref() != Some("istio-waypoint"))
        .collect();
    Ok((external_routes, ingress_gateways))
}

async fn cluster_ingress_gateways(client: &Client) -> Result<Vec<DynamicObject>, RolloutError> {
    let gw_ar = api_resource("gateway.networking.k8s.io", "v1", "Gateway", "gateways");
    let api = Api::<DynamicObject>::all_with(client.clone(), &gw_ar);
    Ok(api
        .list(&kube::api::ListParams::default())
        .await?
        .items)
}

async fn get_gateway(
    client: &Client,
    namespace: &str,
    name: &str,
) -> Result<Option<DynamicObject>, RolloutError> {
    let gw_ar = api_resource("gateway.networking.k8s.io", "v1", "Gateway", "gateways");
    let api = Api::<DynamicObject>::namespaced_with(client.clone(), namespace, &gw_ar);
    match api.get(name).await {
        Ok(gw) => Ok(Some(gw)),
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(None),
        Err(e) => Err(RolloutError::Kube(e)),
    }
}

async fn wait_gateway_programmed(
    client: &Client,
    namespace: &str,
    name: &str,
) -> Result<(), RolloutError> {
    let ar = api_resource("gateway.networking.k8s.io", "v1", "Gateway", "gateways");
    let api = Api::<DynamicObject>::namespaced_with(client.clone(), namespace, &ar);
    let deadline = Duration::from_secs(GATEWAY_READY_TIMEOUT_SECS);
    let started = std::time::Instant::now();
    while started.elapsed() < deadline {
        match api.get(name).await {
            Ok(gw) if gateway_ready(&gw.data) => return Ok(()),
            Ok(_) => {}
            Err(kube::Error::Api(e)) if e.code == 404 => {}
            Err(e) => return Err(RolloutError::Kube(e)),
        }
        sleep(Duration::from_secs(GATEWAY_POLL_INTERVAL_SECS)).await;
    }
    Err(RolloutError::ExecutionFailed(format!(
        "ambient ingress Gateway {namespace}/{name} not programmed within {GATEWAY_READY_TIMEOUT_SECS}s"
    )))
}

async fn patch_httproute_parent_refs(
    client: &Client,
    namespace: &str,
    route_name: &str,
    gw_namespace: &str,
    gw_name: &str,
) -> Result<(), RolloutError> {
    let hr_ar = api_resource("gateway.networking.k8s.io", "v1", "HTTPRoute", "httproutes");
    let api = Api::<DynamicObject>::namespaced_with(client.clone(), namespace, &hr_ar);
    let route = api.get(route_name).await.map_err(RolloutError::Kube)?;
    let original_parents = route
        .data
        .pointer("/spec/parentRefs")
        .cloned()
        .unwrap_or(json!([]));
    let annotations = route.metadata.annotations.clone().unwrap_or_default();
    let mut patch_annotations = annotations.clone();
    if !annotations.contains_key(ORIGINAL_PARENT_REFS_ANNOTATION) {
        patch_annotations.insert(
            ORIGINAL_PARENT_REFS_ANNOTATION.into(),
            original_parents.to_string(),
        );
    }
    let patch = json!({
        "metadata": {
            "annotations": patch_annotations,
            "labels": {
                MANAGED_BY_LABEL: MANAGED_BY_VALUE,
                "ambientor.io/ingress-migrated": "true"
            }
        },
        "spec": {
            "parentRefs": [{
                "group": "gateway.networking.k8s.io",
                "kind": "Gateway",
                "name": gw_name,
                "namespace": gw_namespace,
            }]
        }
    });
    api.patch(route_name, &PatchParams::default(), &Patch::Merge(&patch))
        .await?;
    info!(
        namespace = %namespace,
        route = %route_name,
        gateway = %format!("{gw_namespace}/{gw_name}"),
        "migrated HTTPRoute parentRefs to ambient ingress"
    );
    Ok(())
}

async fn restore_httproute_parent_refs(
    client: &Client,
    namespace: &str,
) -> Result<usize, RolloutError> {
    let hr_ar = api_resource("gateway.networking.k8s.io", "v1", "HTTPRoute", "httproutes");
    let routes = list_cr_in_namespace(client, &hr_ar, namespace)
        .await
        .map_err(|e| RolloutError::ExecutionFailed(e.to_string()))?;
    let api = Api::<DynamicObject>::namespaced_with(client.clone(), namespace, &hr_ar);
    let mut restored = 0usize;
    for route in routes {
        let Some(name) = route.metadata.name else {
            continue;
        };
        let migrated = route
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("ambientor.io/ingress-migrated"))
            .map(String::as_str)
            == Some("true");
        if !migrated {
            continue;
        }
        let original = route
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(ORIGINAL_PARENT_REFS_ANNOTATION))
            .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
            .unwrap_or(json!([]));
        let patch = json!({
            "metadata": {
                "labels": { "ambientor.io/ingress-migrated": null },
                "annotations": { ORIGINAL_PARENT_REFS_ANNOTATION: null }
            },
            "spec": { "parentRefs": original }
        });
        api.patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
            .await?;
        restored += 1;
    }
    Ok(restored)
}

async fn delete_managed_gateway(
    client: &Client,
    namespace: &str,
    name: &str,
) -> Result<bool, RolloutError> {
    let gw_ar = api_resource("gateway.networking.k8s.io", "v1", "Gateway", "gateways");
    let api = Api::<DynamicObject>::namespaced_with(client.clone(), namespace, &gw_ar);
    let gw = match api.get(name).await {
        Ok(g) => g,
        Err(kube::Error::Api(e)) if e.code == 404 => return Ok(false),
        Err(e) => return Err(RolloutError::Kube(e)),
    };
    let managed = gw
        .metadata
        .labels
        .as_ref()
        .is_some_and(|l| {
            l.get(MANAGED_BY_LABEL).map(String::as_str) == Some(MANAGED_BY_VALUE)
                || l.get("ambientor.io/ingress-created").map(String::as_str) == Some("true")
        });
    if !managed {
        return Ok(false);
    }
    api.delete(name, &DeleteParams::default()).await?;
    Ok(true)
}

async fn migrate_openshift_routes(
    client: &Client,
    legacy_services: &[String],
    target_service_namespace: &str,
    target_service_name: &str,
) -> Result<usize, RolloutError> {
    if legacy_services.is_empty() {
        return Ok(0);
    }
    let route_ar = api_resource("route.openshift.io", "v1", "Route", "routes");
    let api = Api::<DynamicObject>::all_with(client.clone(), &route_ar);
    let list = match api.list(&kube::api::ListParams::default()).await {
        Ok(l) => l.items,
        Err(kube::Error::Api(e)) if e.code == 404 => return Ok(0),
        Err(e) => return Err(RolloutError::Kube(e)),
    };

    let legacy: std::collections::BTreeSet<_> = legacy_services.iter().cloned().collect();
    let mut updated = 0usize;
    for route in list {
        let Some(name) = route.metadata.name.clone() else {
            continue;
        };
        let route_ns = route.metadata.namespace.clone().unwrap_or_default();
        let to_name = route
            .data
            .pointer("/spec/to/name")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        if !legacy.contains(to_name) {
            continue;
        }
        let original_to = route.data.pointer("/spec/to").cloned().unwrap_or(json!({}));
        let annotations = route.metadata.annotations.clone().unwrap_or_default();
        let mut patch_annotations = annotations.clone();
        if !annotations.contains_key(ORIGINAL_ROUTE_TARGET_ANNOTATION) {
            patch_annotations.insert(
                ORIGINAL_ROUTE_TARGET_ANNOTATION.into(),
                original_to.to_string(),
            );
        }
        let patch = json!({
            "metadata": {
                "annotations": patch_annotations,
                "labels": {
                    MANAGED_BY_LABEL: MANAGED_BY_VALUE,
                    "ambientor.io/ingress-route-migrated": "true"
                }
            },
            "spec": {
                "to": {
                    "kind": "Service",
                    "name": target_service_name,
                    "namespace": target_service_namespace,
                    "weight": 100
                }
            }
        });
        let route_api =
            Api::<DynamicObject>::namespaced_with(client.clone(), &route_ns, &route_ar);
        route_api
            .patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
            .await?;
        updated += 1;
        info!(
            route = %format!("{route_ns}/{name}"),
            service = %format!("{target_service_namespace}/{target_service_name}"),
            "updated OpenShift Route backend to ambient ingress Service"
        );
    }
    Ok(updated)
}

async fn revert_openshift_routes(client: &Client) -> Result<usize, RolloutError> {
    let route_ar = api_resource("route.openshift.io", "v1", "Route", "routes");
    let api = Api::<DynamicObject>::all_with(client.clone(), &route_ar);
    let list = match api.list(&kube::api::ListParams::default()).await {
        Ok(l) => l.items,
        Err(kube::Error::Api(e)) if e.code == 404 => return Ok(0),
        Err(e) => return Err(RolloutError::Kube(e)),
    };
    let mut reverted = 0usize;
    for route in list {
        let migrated = route
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get("ambientor.io/ingress-route-migrated"))
            .map(String::as_str)
            == Some("true");
        if !migrated {
            continue;
        }
        let Some(name) = route.metadata.name.clone() else {
            continue;
        };
        let route_ns = route.metadata.namespace.clone().unwrap_or_default();
        let original = route
            .metadata
            .annotations
            .as_ref()
            .and_then(|a| a.get(ORIGINAL_ROUTE_TARGET_ANNOTATION))
            .and_then(|v| serde_json::from_str::<serde_json::Value>(v).ok())
            .unwrap_or(json!({}));
        let patch = json!({
            "metadata": {
                "labels": { "ambientor.io/ingress-route-migrated": null },
                "annotations": { ORIGINAL_ROUTE_TARGET_ANNOTATION: null }
            },
            "spec": { "to": original }
        });
        let route_api =
            Api::<DynamicObject>::namespaced_with(client.clone(), &route_ns, &route_ar);
        route_api
            .patch(&name, &PatchParams::default(), &Patch::Merge(&patch))
            .await?;
        reverted += 1;
    }
    Ok(reverted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ambientor_core::rules::{ExternalRouteInfo, IngressGatewayInfo};

    #[test]
    fn detects_route_already_on_target() {
        let target = ResolvedIngressGateway {
            namespace: "bookinfo-demo1".into(),
            name: PER_NAMESPACE_INGRESS_NAME.into(),
            created: true,
        };
        let route = ExternalRouteInfo {
            namespace: "bookinfo-demo1".into(),
            name: "bookinfo".into(),
            kind: "HTTPRoute".into(),
            hostnames: vec!["demo1.example.com".into()],
            parent_gateway_namespace: Some("bookinfo-demo1".into()),
            parent_gateway_name: Some(PER_NAMESPACE_INGRESS_NAME.into()),
            parents_attached: Some(true),
        };
        assert!(route_already_on_target(&route, &target, &[]));
    }

    #[test]
    fn sidecar_route_needs_migration() {
        let target = ResolvedIngressGateway {
            namespace: "bookinfo-demo1".into(),
            name: PER_NAMESPACE_INGRESS_NAME.into(),
            created: true,
        };
        let gateways = vec![IngressGatewayInfo {
            namespace: "bookinfo-gateway".into(),
            name: "demo-gw".into(),
            istio_revision: Some("demo".into()),
            discovery_label: Some("mesh-demo".into()),
            programmed: true,
            gateway_class: Some("istio".into()),
        }];
        let route = ExternalRouteInfo {
            namespace: "bookinfo-demo1".into(),
            name: "bookinfo".into(),
            kind: "HTTPRoute".into(),
            hostnames: vec!["demo1.example.com".into()],
            parent_gateway_namespace: Some("bookinfo-gateway".into()),
            parent_gateway_name: Some("demo-gw".into()),
            parents_attached: Some(false),
        };
        assert!(!route_already_on_target(&route, &target, &gateways));
    }

    #[test]
    fn istio_gateway_service_name_suffix() {
        assert_eq!(
            istio_gateway_service_name("bookinfo-demo-gateway"),
            "bookinfo-demo-gateway-istio"
        );
    }
}
