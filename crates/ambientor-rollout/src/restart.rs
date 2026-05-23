use chrono::Utc;
use k8s_openapi::api::apps::v1::Deployment;
use kube::{
    Api, Client,
    api::{ListParams, Patch, PatchParams},
};
use serde_json::json;
use tracing::info;

use crate::engine::{FIELD_MANAGER, RolloutError};

/// Trigger a rolling restart of all Deployments in a namespace (pod template annotation).
pub async fn rolling_restart_namespace(
    client: &Client,
    namespace: &str,
) -> Result<usize, RolloutError> {
    let api: Api<Deployment> = Api::namespaced(client.clone(), namespace);
    let deps = api.list(&ListParams::default()).await?;
    let restarted_at = Utc::now().to_rfc3339();
    let mut count = 0usize;
    for dep in deps.items {
        let Some(name) = dep.metadata.name else {
            continue;
        };
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
        let pp = PatchParams::apply(FIELD_MANAGER).force();
        api.patch(&name, &pp, &Patch::Apply(patch)).await?;
        count += 1;
        info!(namespace = %namespace, deployment = %name, "rolling restart triggered");
    }
    Ok(count)
}
