//! OpenShift Route helpers — use create/merge patch, never server-side apply on `spec.host`.

use ambientor_mesh::dynamic::api_resource;
use kube::api::{DynamicObject, Patch, PatchParams, PostParams};
use kube::{Api, Client};
use serde_json::Value;

use crate::engine::RolloutError;

const ROUTE_API_VERSION: &str = "route.openshift.io/v1";
const ROUTE_KIND: &str = "Route";

fn route_api(client: &Client, namespace: &str) -> Api<DynamicObject> {
    let ar = api_resource("route.openshift.io", "v1", "Route", "routes");
    Api::<DynamicObject>::namespaced_with(client.clone(), namespace, &ar)
}

fn route_name(manifest: &Value) -> Result<&str, RolloutError> {
    manifest
        .pointer("/metadata/name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RolloutError::ExecutionFailed("route manifest missing metadata.name".into()))
}

/// Create a Route or merge-patch backend/labels on an existing Route (never changes `spec.host`).
pub async fn upsert_openshift_route(
    client: &Client,
    namespace: &str,
    manifest: &Value,
) -> Result<String, RolloutError> {
    let name = route_name(manifest)?;
    let api = route_api(client, namespace);

    match api.get(name).await {
        Ok(existing) => {
            let existing_to = existing
                .data
                .pointer("/spec/to/name")
                .and_then(|v| v.as_str());
            let target_to = manifest
                .pointer("/spec/to/name")
                .and_then(|v| v.as_str());
            if existing_to == target_to {
                if let Some(labels) = manifest.pointer("/metadata/labels") {
                    api.patch(
                        name,
                        &PatchParams::default(),
                        &Patch::Merge(&serde_json::json!({
                            "metadata": { "labels": labels }
                        })),
                    )
                    .await?;
                }
            } else {
                let mut patch = serde_json::json!({
                    "metadata": {},
                    "spec": {}
                });
                if let Some(labels) = manifest.pointer("/metadata/labels") {
                    patch["metadata"]["labels"] = labels.clone();
                }
                if let Some(to) = manifest.pointer("/spec/to") {
                    patch["spec"]["to"] = to.clone();
                }
                if let Some(port) = manifest.pointer("/spec/port") {
                    patch["spec"]["port"] = port.clone();
                }
                if let Some(tls) = manifest.pointer("/spec/tls") {
                    patch["spec"]["tls"] = tls.clone();
                }
                api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
                    .await?;
            }
        }
        Err(kube::Error::Api(e)) if e.code == 404 => {
            let obj: DynamicObject = serde_json::from_value(manifest.clone()).map_err(|err| {
                RolloutError::ExecutionFailed(format!("invalid Route/{name}: {err}"))
            })?;
            api.create(&PostParams::default(), &obj).await?;
        }
        Err(e) => return Err(RolloutError::Kube(e)),
    }
    Ok(format!("{ROUTE_KIND}/{name}"))
}

/// Restore a Route snapshot (create if missing; merge-patch backend if present).
pub async fn restore_openshift_route_snapshot(
    client: &Client,
    snapshot: &Value,
) -> Result<(), RolloutError> {
    let ns = snapshot
        .pointer("/metadata/namespace")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RolloutError::ExecutionFailed("route snapshot missing namespace".into()))?;
    let name = snapshot
        .pointer("/metadata/name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| RolloutError::ExecutionFailed("route snapshot missing name".into()))?;

    let mut restore = snapshot.clone();
    if let Some(meta) = restore.get_mut("metadata").and_then(|m| m.as_object_mut()) {
        meta.remove("resourceVersion");
        meta.remove("uid");
        meta.remove("creationTimestamp");
        meta.remove("generation");
        meta.insert("namespace".into(), serde_json::json!(ns));
        meta.insert("name".into(), serde_json::json!(name));
    }
    restore
        .as_object_mut()
        .expect("restore object")
        .insert("apiVersion".into(), serde_json::json!(ROUTE_API_VERSION));
    restore
        .as_object_mut()
        .expect("restore object")
        .insert("kind".into(), serde_json::json!(ROUTE_KIND));

    upsert_openshift_route(client, ns, &restore).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_name_from_manifest() {
        let m = serde_json::json!({ "metadata": { "name": "demo3-bookinfo" } });
        assert_eq!(route_name(&m).unwrap(), "demo3-bookinfo");
    }
}
