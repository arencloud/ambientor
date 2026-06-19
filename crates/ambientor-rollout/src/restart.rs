use chrono::Utc;
use k8s_openapi::api::apps::v1::Deployment;
use kube::{
    Api, Client,
    api::{ListParams, Patch, PatchParams},
};
use serde_json::json;
use tracing::info;

use crate::engine::RolloutError;

use crate::ingress::PER_NAMESPACE_INGRESS_NAME;

/// Trigger a rolling restart of application Deployments in a namespace (pod template annotation).
/// Skips Istio gateway Deployments created for ambient ingress or waypoint.
pub async fn rolling_restart_namespace(
    client: &Client,
    namespace: &str,
) -> Result<usize, RolloutError> {
    let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    let deps = api.list(&ListParams::default()).await?;
    let restarted_at = Utc::now().to_rfc3339();
    let mut count = 0usize;
    for dep in deps.items {
        let Some(name) = dep.metadata.name.as_deref() else {
            continue;
        };
        if skip_rolling_restart(name, &dep) {
            continue;
        }
        let patch = json!({
            "spec": {
                "template": {
                    "metadata": {
                        "annotations": {
                            "ambientor.io/restartedAt": restarted_at
                        }
                    }
                }
            }
        });
        // Merge patch: SSA Apply requires apiVersion/kind on the patch body.
        api.patch(name, &PatchParams::default(), &Patch::Merge(&patch))
            .await?;
        count += 1;
        info!(namespace = %namespace, deployment = %name, "rolling restart triggered");
    }
    Ok(count)
}

fn skip_rolling_restart(name: &str, dep: &Deployment) -> bool {
    if name == "waypoint-istio" || name.ends_with("-istio") && name.starts_with(PER_NAMESPACE_INGRESS_NAME) {
        return true;
    }
    let labels = dep.metadata.labels.as_ref();
    labels.is_some_and(|l| {
        l.get("ambientor.io/ingress-created").map(String::as_str) == Some("true")
            || l.get("gateway.istio.io/managed").map(String::as_str) == Some("istio.io-gateway-controller")
            || l.get("gateway.networking.k8s.io/gateway-name").is_some()
    })
}
