//! Suggest Gateway API HTTPRoute manifests from Istio VirtualService resources.

use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranslationResult {
    pub manifest: String,
    pub warnings: Vec<String>,
}

/// Convert a VirtualService `spec` JSON object into a suggested HTTPRoute manifest (YAML).
pub fn virtual_service_to_httproute(
    namespace: &str,
    vs_name: &str,
    vs_data: &Value,
) -> Result<TranslationResult, String> {
    let spec = vs_data
        .get("spec")
        .ok_or_else(|| "VirtualService missing spec".to_string())?;

    if spec.get("tcp").is_some() || spec.get("tls").is_some() {
        return Err("TCP/TLS VirtualService routes are not translated to HTTPRoute".into());
    }

    let mut warnings = Vec::new();
    warnings
        .push("parentRefs are omitted; attach this HTTPRoute to your Gateway or waypoint.".into());

    let (hostnames, host_warnings) = httproute_hostnames_from_spec(spec);
    warnings.extend(host_warnings);
    if hostnames.is_empty() {
        warnings.push(
            "HTTPRoute spec.hostnames omitted (empty matches all hostnames, same as Istio wildcard)."
                .into(),
        );
    }

    let rules = http_rules_from_spec(spec, &mut warnings);
    if rules.is_empty() {
        return Err("No translatable spec.http routes found".into());
    }

    let route_name = format!("{vs_name}-ambientor");
    let mut spec = json!({ "rules": rules });
    if !hostnames.is_empty() {
        spec["hostnames"] = json!(hostnames);
    }
    let manifest_value = json!({
        "apiVersion": "gateway.networking.k8s.io/v1",
        "kind": "HTTPRoute",
        "metadata": {
            "name": route_name,
            "namespace": namespace,
            "labels": {
                "ambientor.io/translated-from": "VirtualService",
                "ambientor.io/source-name": vs_name,
            }
        },
        "spec": spec,
    });

    let manifest = serde_yaml::to_string(&manifest_value)
        .map_err(|e| format!("failed to serialize suggested manifest: {e}"))?;

    Ok(TranslationResult { manifest, warnings })
}

/// Gateway API hostnames must match a DNS subdomain pattern; bare `*` is invalid.
fn is_valid_httproute_hostname(host: &str) -> bool {
    if host == "*" {
        return false;
    }
    // `*.example.com` and `reviews.bookinfo.svc.cluster.local` style names.
    host.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '*')
        && !host.is_empty()
}

/// Map Istio `spec.hosts` to HTTPRoute `hostnames`, dropping values the API rejects.
fn httproute_hostnames_from_spec(spec: &Value) -> (Vec<Value>, Vec<String>) {
    let mut warnings = Vec::new();
    let hosts: Vec<&str> = spec
        .get("hosts")
        .and_then(|h| h.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();

    let mut hostnames = Vec::new();
    for host in hosts {
        if is_valid_httproute_hostname(host) {
            hostnames.push(json!(host));
        } else {
            warnings.push(format!(
                "spec.hosts entry {host:?} is not a valid HTTPRoute hostname; omitted (use empty hostnames for wildcard)"
            ));
        }
    }
    (hostnames, warnings)
}

fn http_rules_from_spec(spec: &Value, warnings: &mut Vec<String>) -> Vec<Value> {
    let Some(http) = spec.get("http").and_then(|h| h.as_array()) else {
        return vec![];
    };

    let mut rules = Vec::new();
    for (i, route) in http.iter().enumerate() {
        if let Some(rule) = translate_http_block(route, i, warnings) {
            rules.push(rule);
        }
    }
    rules
}

fn translate_http_block(route: &Value, index: usize, warnings: &mut Vec<String>) -> Option<Value> {
    if route.get("redirect").is_some() {
        return translate_redirect_block(route, index, warnings);
    }

    let matches = uri_matches(route, index, warnings);
    let backend_refs = backend_refs_from_route(route, index, warnings);

    if backend_refs.is_empty() {
        warnings.push(format!(
            "spec.http[{index}]: no destination host found; rule skipped"
        ));
        return None;
    }

    let mut rule = json!({ "backendRefs": backend_refs });
    if !matches.is_empty() {
        rule["matches"] = json!(matches);
    }
    Some(rule)
}

fn translate_redirect_block(
    route: &Value,
    index: usize,
    warnings: &mut Vec<String>,
) -> Option<Value> {
    let redirect = route.get("redirect")?;
    let matches = uri_matches(route, index, warnings);
    let mut request_redirect = json!({});
    if let Some(scheme) = redirect.get("scheme").and_then(|v| v.as_str()) {
        request_redirect["scheme"] = json!(scheme);
    }
    if let Some(uri) = redirect.get("uri").and_then(|v| v.as_str()) {
        request_redirect["path"] = json!({
            "type": "ReplaceFullPath",
            "replaceFullPath": uri,
        });
    }
    if let Some(code) = redirect.get("redirectCode").and_then(|v| v.as_u64()) {
        request_redirect["statusCode"] = json!(code);
    } else {
        request_redirect["statusCode"] = json!(302);
    }
    let mut rule = json!({
        "filters": [{
            "type": "RequestRedirect",
            "requestRedirect": request_redirect,
        }]
    });
    if !matches.is_empty() {
        rule["matches"] = json!(matches);
    } else {
        warnings.push(format!(
            "spec.http[{index}]: redirect rule has no URI matches; applies to all paths on host"
        ));
    }
    Some(rule)
}

fn uri_matches(route: &Value, index: usize, warnings: &mut Vec<String>) -> Vec<Value> {
    let Some(match_arr) = route.get("match").and_then(|m| m.as_array()) else {
        return vec![];
    };

    let mut out = Vec::new();
    for m in match_arr {
        if let Some(uri) = m.get("uri") {
            if let Some(prefix) = uri.get("prefix").and_then(|p| p.as_str()) {
                out.push(json!({
                    "path": { "type": "PathPrefix", "value": prefix }
                }));
            } else if let Some(exact) = uri.get("exact").and_then(|e| e.as_str()) {
                out.push(json!({
                    "path": { "type": "Exact", "value": exact }
                }));
            } else if let Some(regex) = uri.get("regex").and_then(|r| r.as_str()) {
                warnings.push(format!(
                    "spec.http[{index}]: URI regex {regex:?} not mapped; review manually"
                ));
            }
        }
    }
    out
}

fn backend_refs_from_route(route: &Value, index: usize, warnings: &mut Vec<String>) -> Vec<Value> {
    let Some(dests) = route.get("route").and_then(|r| r.as_array()) else {
        return vec![];
    };

    let mut refs = Vec::new();
    for dest in dests {
        let Some(host) = dest
            .get("destination")
            .and_then(|d| d.get("host"))
            .and_then(|h| h.as_str())
        else {
            continue;
        };
        let service_name = normalize_backend_service_name(host);
        let mut backend = json!({ "name": service_name });
        if let Some(port) = dest
            .get("destination")
            .and_then(|d| d.get("port"))
            .and_then(|p| p.get("number"))
            .and_then(|n| n.as_u64())
        {
            backend["port"] = json!(port);
        } else {
            warnings.push(format!(
                "spec.http[{index}]: destination {host} has no port.number; omitting port"
            ));
        }
        if let Some(weight) = dest.get("weight").and_then(|w| w.as_u64())
            && weight != 100
        {
            warnings.push(format!(
                "spec.http[{index}]: weight {weight} not represented on backendRef"
            ));
        }
        refs.push(backend);
    }
    refs
}

/// Map Istio destination hosts to Kubernetes Service names for backendRefs.
fn normalize_backend_service_name(host: &str) -> String {
    host.split('.').next().unwrap_or(host).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn translates_prefix_route() {
        let vs = json!({
            "spec": {
                "hosts": ["reviews"],
                "http": [{
                    "match": [{ "uri": { "prefix": "/reviews" } }],
                    "route": [{
                        "destination": { "host": "reviews", "port": { "number": 9080 } }
                    }]
                }]
            }
        });
        let result = virtual_service_to_httproute("bookinfo", "reviews", &vs).unwrap();
        assert!(result.manifest.contains("kind: HTTPRoute"));
        assert!(result.manifest.contains("PathPrefix"));
        assert!(result.manifest.contains("/reviews"));
        assert!(result.manifest.contains("reviews-ambientor"));
        assert!(result.warnings.iter().any(|w| w.contains("parentRefs")));
    }

    #[test]
    fn omits_bare_wildcard_host() {
        let vs = json!({
            "spec": {
                "hosts": ["*"],
                "http": [{
                    "route": [{
                        "destination": { "host": "sidecar-app", "port": { "number": 8080 } }
                    }]
                }]
            }
        });
        let result = virtual_service_to_httproute("mesh-sidecar-2", "sidecar-app-vs", &vs).unwrap();
        assert!(!result.manifest.contains("hostnames:"));
        assert!(result.warnings.iter().any(|w| w.contains("wildcard")));
    }

    #[test]
    fn rejects_tcp_only() {
        let vs = json!({ "spec": { "tcp": [{ "route": [] }] } });
        assert!(virtual_service_to_httproute("ns", "svc", &vs).is_err());
    }

    #[test]
    fn normalizes_fqdn_destination_to_service_name() {
        let vs = json!({
            "spec": {
                "hosts": ["demo4.apps.example.com"],
                "http": [{
                    "route": [{
                        "destination": {
                            "host": "productpage.bookinfo-demo4.svc.cluster.local",
                            "port": { "number": 9080 }
                        }
                    }]
                }]
            }
        });
        let result = virtual_service_to_httproute("bookinfo-demo4", "bookinfo", &vs).unwrap();
        assert!(result.manifest.contains("name: productpage"));
        assert!(!result.manifest.contains("svc.cluster.local"));
    }

    #[test]
    fn translates_redirect_rule() {
        let vs = json!({
            "spec": {
                "hosts": ["demo4.apps.example.com"],
                "http": [
                    {
                        "match": [{ "uri": { "exact": "/" } }],
                        "redirect": {
                            "redirectCode": 302,
                            "scheme": "https",
                            "uri": "/productpage"
                        }
                    },
                    {
                        "route": [{
                            "destination": {
                                "host": "productpage",
                                "port": { "number": 9080 }
                            }
                        }]
                    }
                ]
            }
        });
        let result = virtual_service_to_httproute("bookinfo-demo4", "bookinfo", &vs).unwrap();
        assert!(result.manifest.contains("RequestRedirect"));
        assert!(result.manifest.contains("/productpage"));
    }
}
