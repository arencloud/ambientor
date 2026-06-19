use std::collections::BTreeMap;

use ambientor_core::rules::{ExternalRouteInfo, IngressGatewayInfo};
use k8s_openapi::api::core::v1::Namespace;
use kube::api::DynamicObject;
use serde_json::Value;

pub fn build_ingress_context(
    gateways: &[DynamicObject],
    namespaces: &[Namespace],
    http_routes: &[DynamicObject],
    virtual_services: &[DynamicObject],
) -> (Vec<IngressGatewayInfo>, Vec<ExternalRouteInfo>) {
    let ns_labels: BTreeMap<String, BTreeMap<String, String>> = namespaces
        .iter()
        .filter_map(|ns| {
            let name = ns.metadata.name.clone()?;
            Some((name, ns.metadata.labels.clone().unwrap_or_default()))
        })
        .collect();

    let ingress_gateways = gateways
        .iter()
        .filter_map(|gw| parse_ingress_gateway(gw, &ns_labels))
        .collect();

    let mut external_routes = Vec::new();
    for route in http_routes {
        if let Some(info) = parse_http_route(route) {
            external_routes.push(info);
        }
    }
    for vs in virtual_services {
        if let Some(info) = parse_virtual_service(vs) {
            external_routes.push(info);
        }
    }

    (ingress_gateways, external_routes)
}

fn parse_ingress_gateway(
    gw: &DynamicObject,
    ns_labels: &BTreeMap<String, BTreeMap<String, String>>,
) -> Option<IngressGatewayInfo> {
    let ns = gw.metadata.namespace.clone()?;
    let name = gw.metadata.name.clone()?;
    let labels = gw.metadata.labels.clone().unwrap_or_default();
    let gateway_class = gw
        .data
        .get("spec")
        .and_then(|s| s.get("gatewayClassName"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    // Waypoint gateways are not north–south ingress.
    if gateway_class.as_deref() == Some("istio-waypoint") {
        return None;
    }
    let ns_label_map = ns_labels.get(&ns);
    let istio_revision = labels
        .get("istio.io/rev")
        .cloned()
        .or_else(|| ns_label_map.and_then(|l| l.get("istio.io/rev").cloned()));
    let discovery_label = labels
        .get("istio-discovery")
        .cloned()
        .or_else(|| ns_label_map.and_then(|l| l.get("istio-discovery").cloned()));
    let programmed = gateway_programmed(&gw.data);
    Some(IngressGatewayInfo {
        namespace: ns,
        name,
        istio_revision,
        discovery_label,
        programmed,
        gateway_class,
    })
}

fn gateway_programmed(data: &Value) -> bool {
    if data
        .get("status")
        .and_then(|s| s.get("addresses"))
        .and_then(|a| a.as_array())
        .is_some_and(|a| !a.is_empty())
    {
        return true;
    }
    let conditions = data
        .get("status")
        .and_then(|s| s.get("conditions"))
        .and_then(|c| c.as_array());
    let Some(conds) = conditions else {
        return false;
    };
    conds.iter().any(|c| {
        let t = c.get("type").and_then(|t| t.as_str());
        let s = c.get("status").and_then(|s| s.as_str());
        (t == Some("Programmed") && s == Some("True"))
            || (t == Some("Accepted") && s == Some("True"))
    })
}

fn parse_http_route(route: &DynamicObject) -> Option<ExternalRouteInfo> {
    let ns = route.metadata.namespace.clone()?;
    let name = route.metadata.name.clone()?;
    let spec = route.data.get("spec")?;
    let hostnames: Vec<String> = spec
        .get("hostnames")
        .and_then(|h| h.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let parent = spec
        .get("parentRefs")
        .and_then(|p| p.as_array())
        .and_then(|arr| arr.first());
    let parent_gateway_namespace = parent
        .and_then(|p| p.get("namespace"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .or_else(|| parent.map(|_| ns.clone()));
    let parent_gateway_name = parent
        .and_then(|p| p.get("name"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    if hostnames.is_empty() && parent_gateway_name.is_none() {
        return None;
    }
    let parents_attached = route
        .data
        .get("status")
        .and_then(|s| s.get("parents"))
        .and_then(|p| p.as_array())
        .map(|arr| !arr.is_empty());
    Some(ExternalRouteInfo {
        namespace: ns,
        name,
        kind: "HTTPRoute".into(),
        hostnames,
        parent_gateway_namespace,
        parent_gateway_name,
        parents_attached,
    })
}

fn parse_virtual_service(vs: &DynamicObject) -> Option<ExternalRouteInfo> {
    let ns = vs.metadata.namespace.clone()?;
    let name = vs.metadata.name.clone()?;
    let spec = vs.data.get("spec")?;
    let hostnames: Vec<String> = spec
        .get("hosts")
        .and_then(|h| h.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let gateways = spec.get("gateways").and_then(|g| g.as_array())?;
    let first = gateways.first()?.as_str()?;
    let (parent_gateway_namespace, parent_gateway_name) = if let Some((gns, gname)) = first.split_once('/')
    {
        (Some(gns.to_string()), Some(gname.to_string()))
    } else {
        (Some(ns.clone()), Some(first.to_string()))
    };
    if hostnames.is_empty() {
        return None;
    }
    Some(ExternalRouteInfo {
        namespace: ns,
        name,
        kind: "VirtualService".into(),
        hostnames,
        parent_gateway_namespace,
        parent_gateway_name,
        parents_attached: None,
    })
}

fn mesh_is_ambient(revision: Option<&str>, discovery: Option<&str>) -> bool {
    discovery.is_some_and(|d| d.contains("ambient"))
        || revision.is_some_and(|r| r.contains("ambient"))
}

fn mesh_is_sidecar_legacy(revision: Option<&str>, discovery: Option<&str>) -> bool {
    if mesh_is_ambient(revision, discovery) {
        return false;
    }
    discovery.is_some_and(|d| d.contains("demo") || d.contains("sidecar"))
        || revision.is_some_and(|r| !r.is_empty())
}

pub fn gateway_for_route<'a>(
    route: &ExternalRouteInfo,
    gateways: &'a [IngressGatewayInfo],
) -> Option<&'a IngressGatewayInfo> {
    let gns = route.parent_gateway_namespace.as_deref()?;
    let gname = route.parent_gateway_name.as_deref()?;
    gateways
        .iter()
        .find(|g| g.namespace == gns && g.name == gname)
}

pub fn has_programmed_ambient_ingress(gateways: &[IngressGatewayInfo]) -> bool {
    gateways.iter().any(|g| {
        g.programmed
            && mesh_is_ambient(
                g.istio_revision.as_deref(),
                g.discovery_label.as_deref(),
            )
    })
}

pub fn route_uses_sidecar_ingress(
    route: &ExternalRouteInfo,
    gateways: &[IngressGatewayInfo],
) -> bool {
    let Some(gw) = gateway_for_route(route, gateways) else {
        return route.parent_gateway_name.is_some();
    };
    mesh_is_sidecar_legacy(
        gw.istio_revision.as_deref(),
        gw.discovery_label.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::api::DynamicObject;
    use serde_json::json;

    #[test]
    fn detects_sidecar_gateway_and_detached_httproute() {
        let gw: DynamicObject = serde_json::from_value(json!({
            "apiVersion": "gateway.networking.k8s.io/v1",
            "kind": "Gateway",
            "metadata": {
                "name": "demo-gw",
                "namespace": "bookinfo-gateway",
                "labels": { "istio.io/rev": "demo" }
            },
            "spec": { "gatewayClassName": "istio" },
            "status": { "conditions": [{ "type": "Programmed", "status": "False" }] }
        }))
        .unwrap();
        let hr: DynamicObject = serde_json::from_value(json!({
            "apiVersion": "gateway.networking.k8s.io/v1",
            "kind": "HTTPRoute",
            "metadata": { "name": "bookinfo", "namespace": "bookinfo-demo1" },
            "spec": {
                "hostnames": ["demo1.example.com"],
                "parentRefs": [{ "name": "demo-gw", "namespace": "bookinfo-gateway" }]
            },
            "status": { "parents": [] }
        }))
        .unwrap();
        let (gateways, routes) = build_ingress_context(&[gw], &[], &[hr], &[]);
        assert_eq!(gateways.len(), 1);
        assert_eq!(routes.len(), 1);
        assert!(route_uses_sidecar_ingress(&routes[0], &gateways));
        assert_eq!(routes[0].parents_attached, Some(false));
    }

    #[test]
    fn accepted_ambient_gateway_counts_as_operational() {
        let gw: DynamicObject = serde_json::from_value(json!({
            "apiVersion": "gateway.networking.k8s.io/v1",
            "kind": "Gateway",
            "metadata": {
                "name": "ambient-ingress",
                "namespace": "bookinfo-demo1",
                "labels": { "istio-discovery": "mesh-ambient", "istio.io/rev": "ambient-v1-28-6" }
            },
            "spec": { "gatewayClassName": "istio" },
            "status": {
                "conditions": [
                    { "type": "Accepted", "status": "True" },
                    { "type": "Programmed", "status": "False" }
                ]
            }
        }))
        .unwrap();
        let (gateways, _) = build_ingress_context(&[gw], &[], &[], &[]);
        assert!(has_programmed_ambient_ingress(&gateways));
    }
}
