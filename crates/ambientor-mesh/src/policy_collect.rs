use ambientor_core::rules::PolicyContext;
use kube::api::DynamicObject;
use serde_json::Value;

use crate::dynamic::resource_ref;

pub fn build_policy_context(objects: &IstioPolicyObjects) -> PolicyContext {
    PolicyContext {
        peer_auth_disable: objects
            .peer_authentications
            .iter()
            .filter(|o| peer_auth_is_disable(o))
            .map(resource_ref)
            .collect(),
        l7_authorization_policies: objects
            .authorization_policies
            .iter()
            .filter(|o| authorization_policy_has_l7(o))
            .map(resource_ref)
            .collect(),
        virtual_services: objects.virtual_services.iter().map(resource_ref).collect(),
        http_routes: objects.http_routes.iter().map(resource_ref).collect(),
        envoy_filters: objects.envoy_filters.iter().map(resource_ref).collect(),
    }
}

pub struct IstioPolicyObjects {
    pub peer_authentications: Vec<DynamicObject>,
    pub authorization_policies: Vec<DynamicObject>,
    pub virtual_services: Vec<DynamicObject>,
    pub envoy_filters: Vec<DynamicObject>,
    pub http_routes: Vec<DynamicObject>,
    pub wasm_plugins: Vec<DynamicObject>,
}

fn peer_auth_is_disable(obj: &DynamicObject) -> bool {
    obj.data
        .get("spec")
        .and_then(|s| s.get("mtls"))
        .and_then(|m| m.get("mode"))
        .and_then(|v| v.as_str())
        .is_some_and(|m| m.eq_ignore_ascii_case("DISABLE"))
}

fn authorization_policy_has_l7(obj: &DynamicObject) -> bool {
    let spec = obj.data.get("spec");
    let Some(spec) = spec else {
        return false;
    };
    if spec.get("rules").is_some_and(rule_list_has_http) {
        return true;
    }
    if spec
        .get("action")
        .and_then(|v| v.as_str())
        .is_some_and(|a| a == "DENY" || a == "ALLOW")
    {
        return spec.get("rules").map(rule_list_has_http).unwrap_or(false);
    }
    false
}

fn rule_list_has_http(rules: &Value) -> bool {
    let Some(arr) = rules.as_array() else {
        return false;
    };
    arr.iter().any(|rule| {
        rule.get("to")
            .and_then(|v| v.as_array())
            .is_some_and(|tos| tos.iter().any(to_has_http))
            || rule
                .get("from")
                .and_then(|v| v.as_array())
                .is_some_and(|froms| froms.iter().any(from_has_http))
    })
}

fn to_has_http(to: &Value) -> bool {
    to.get("operation").map(operation_has_http).unwrap_or(false)
}

fn from_has_http(from: &Value) -> bool {
    from.get("operation")
        .map(operation_has_http)
        .unwrap_or(false)
}

fn operation_has_http(op: &Value) -> bool {
    op.as_object().is_some_and(|o| {
        o.contains_key("paths")
            || o.contains_key("methods")
            || o.contains_key("hosts")
            || o.contains_key("ports")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kube::api::DynamicObject;
    use serde_json::json;

    fn obj(data: serde_json::Value) -> DynamicObject {
        serde_json::from_value(json!({
            "apiVersion": "security.istio.io/v1",
            "kind": "PeerAuthentication",
            "metadata": { "name": "test", "namespace": "bookinfo" },
            "spec": data
        }))
        .expect("dynamic object")
    }

    #[test]
    fn detects_peer_auth_disable() {
        let o = obj(json!({ "mtls": { "mode": "DISABLE" } }));
        assert!(peer_auth_is_disable(&o));
    }

    #[test]
    fn detects_l7_authorization_policy() {
        let data = json!({
            "apiVersion": "security.istio.io/v1",
            "kind": "AuthorizationPolicy",
            "metadata": { "name": "httpbin", "namespace": "foo" },
            "spec": {
                "rules": [{
                    "to": [{ "operation": { "paths": ["/admin"] } }]
                }]
            }
        });
        let o: DynamicObject = serde_json::from_value(data).unwrap();
        assert!(authorization_policy_has_l7(&o));
    }
}
