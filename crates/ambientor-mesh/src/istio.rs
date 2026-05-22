use kube::Client;

use crate::dynamic::{api_resource, list_namespaced_cr};
use crate::policy_collect::IstioPolicyObjects;

pub async fn collect_istio_policies(client: &Client) -> anyhow::Result<IstioPolicyObjects> {
    let peer_ar = api_resource(
        "security.istio.io",
        "v1",
        "PeerAuthentication",
        "peerauthentications",
    );
    let auth_ar = api_resource(
        "security.istio.io",
        "v1",
        "AuthorizationPolicy",
        "authorizationpolicies",
    );
    let vs_ar = api_resource(
        "networking.istio.io",
        "v1",
        "VirtualService",
        "virtualservices",
    );
    let ef_ar = api_resource("networking.istio.io", "v1", "EnvoyFilter", "envoyfilters");
    let wasm_ar = api_resource("extensions.istio.io", "v1", "WasmPlugin", "wasmplugins");
    let hr_ar = api_resource("gateway.networking.k8s.io", "v1", "HTTPRoute", "httproutes");
    let dr_ar = api_resource(
        "networking.istio.io",
        "v1",
        "DestinationRule",
        "destinationrules",
    );

    Ok(IstioPolicyObjects {
        peer_authentications: list_namespaced_cr(client, &peer_ar)
            .await
            .unwrap_or_default(),
        authorization_policies: list_namespaced_cr(client, &auth_ar)
            .await
            .unwrap_or_default(),
        virtual_services: list_namespaced_cr(client, &vs_ar).await.unwrap_or_default(),
        envoy_filters: list_namespaced_cr(client, &ef_ar).await.unwrap_or_default(),
        wasm_plugins: list_namespaced_cr(client, &wasm_ar)
            .await
            .unwrap_or_default(),
        http_routes: list_namespaced_cr(client, &hr_ar).await.unwrap_or_default(),
        destination_rules: list_namespaced_cr(client, &dr_ar).await.unwrap_or_default(),
    })
}
