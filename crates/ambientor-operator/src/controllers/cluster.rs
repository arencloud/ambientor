use std::sync::Arc;

use ambientor_k8s::detect_platform;
use ambientor_types::Cluster;
use futures::StreamExt;
use kube::{
    Api, Client,
    api::Patch,
    runtime::controller::{Action, Controller},
    runtime::watcher::Config,
};

use super::runtime::{ReconcileError, ReconcileResult, error_policy};

pub async fn run(client: Client) {
    Controller::new(Api::<Cluster>::all(client.clone()), Config::default())
        .shutdown_on_signal()
        .run(reconcile, error_policy, Arc::new(client))
        .for_each(|res| async move {
            if let Err(e) = res {
                tracing::error!(error = ?e, "cluster controller error");
            }
        })
        .await;
}

async fn reconcile(obj: Arc<Cluster>, client: Arc<Client>) -> ReconcileResult {
    let platform = detect_platform(client.as_ref()).await.unwrap_or_default();
    let api: Api<Cluster> = Api::all(client.as_ref().clone());
    if let Some(name) = &obj.metadata.name {
        let status = serde_json::json!({
            "status": {
                "phase": "Ready",
                "meshVersion": platform.version,
            }
        });
        api.patch_status(name, &Default::default(), &Patch::Merge(status))
            .await
            .map_err(ReconcileError::Kube)?;
    }
    Ok(Action::await_change())
}
