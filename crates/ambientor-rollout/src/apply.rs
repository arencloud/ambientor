use ambientor_mesh::dynamic::api_resource;
use kube::api::{ApiResource, DynamicObject, Patch, PatchParams};
use kube::{Api, Client};
use serde_json::Value;

use crate::engine::FIELD_MANAGER;
use crate::engine::RolloutError;

/// Apply a namespaced manifest (JSON) with server-side apply.
pub async fn apply_namespaced_manifest(
    client: &Client,
    namespace: &str,
    manifest: &Value,
) -> Result<String, RolloutError> {
    let api_version = manifest
        .get("apiVersion")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RolloutError::ExecutionFailed("manifest missing apiVersion".into()))?;
    let kind = manifest
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RolloutError::ExecutionFailed("manifest missing kind".into()))?;
    let name = manifest
        .pointer("/metadata/name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RolloutError::ExecutionFailed("manifest missing metadata.name".into()))?;

    let ar = api_resource_from_version_kind(api_version, kind)?;
    let api = Api::<DynamicObject>::namespaced_with(client.clone(), namespace, &ar);
    let obj: DynamicObject = serde_json::from_value(manifest.clone()).map_err(|e| {
        RolloutError::ExecutionFailed(format!("invalid manifest for {kind}/{name}: {e}"))
    })?;
    let pp = PatchParams::apply(FIELD_MANAGER).force();
    api.patch(name, &pp, &Patch::Apply(&obj)).await?;
    Ok(format!("{kind}/{name}"))
}

fn api_resource_from_version_kind(
    api_version: &str,
    kind: &str,
) -> Result<ApiResource, RolloutError> {
    match (api_version, kind) {
        ("gateway.networking.k8s.io/v1", "Gateway") => Ok(api_resource(
            "gateway.networking.k8s.io",
            "v1",
            "Gateway",
            "gateways",
        )),
        ("gateway.networking.k8s.io/v1", "HTTPRoute") => Ok(api_resource(
            "gateway.networking.k8s.io",
            "v1",
            "HTTPRoute",
            "httproutes",
        )),
        ("gateway.networking.k8s.io/v1beta1", "Gateway") => Ok(api_resource(
            "gateway.networking.k8s.io",
            "v1beta1",
            "Gateway",
            "gateways",
        )),
        ("route.openshift.io/v1", "Route") => Ok(api_resource(
            "route.openshift.io",
            "v1",
            "Route",
            "routes",
        )),
        _ => Err(RolloutError::ExecutionFailed(format!(
            "unsupported manifest {api_version} {kind}"
        ))),
    }
}
