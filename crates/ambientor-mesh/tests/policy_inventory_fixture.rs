use ambientor_mesh::policy_collect::{IstioPolicyObjects, build_policy_context};
use kube::api::DynamicObject;
use serde_json::json;

#[test]
fn builds_policy_context_from_fixtures() {
    let peer: DynamicObject = serde_json::from_value(json!({
        "apiVersion": "security.istio.io/v1",
        "kind": "PeerAuthentication",
        "metadata": { "name": "default", "namespace": "bookinfo" },
        "spec": { "mtls": { "mode": "DISABLE" } }
    }))
    .unwrap();
    let vs: DynamicObject = serde_json::from_value(json!({
        "apiVersion": "networking.istio.io/v1",
        "kind": "VirtualService",
        "metadata": { "name": "reviews", "namespace": "bookinfo" },
        "spec": {}
    }))
    .unwrap();
    let hr: DynamicObject = serde_json::from_value(json!({
        "apiVersion": "gateway.networking.k8s.io/v1",
        "kind": "HTTPRoute",
        "metadata": { "name": "reviews", "namespace": "bookinfo" },
        "spec": {}
    }))
    .unwrap();

    let ctx = build_policy_context(
        &IstioPolicyObjects {
        peer_authentications: vec![peer],
        authorization_policies: vec![],
        virtual_services: vec![vs],
        envoy_filters: vec![],
        http_routes: vec![hr],
        gateways: vec![],
        wasm_plugins: vec![],
        destination_rules: vec![],
    },
        &[],
    );

    assert_eq!(ctx.peer_auth_disable.len(), 1);
    assert_eq!(ctx.virtual_services.len(), 1);
    assert_eq!(ctx.http_routes.len(), 1);
}
