use std::sync::Arc;

use ambientor_types::ClusterConnection;
use futures::StreamExt;
use k8s_openapi::api::core::v1::Secret;
use kube::{
    Api, Client,
    api::Patch,
    runtime::controller::{Action, Controller},
    runtime::watcher::Config,
};

use super::runtime::{ReconcileError, ReconcileResult, error_policy};

pub async fn run(client: Client) {
    Controller::new(
        Api::<ClusterConnection>::all(client.clone()),
        Config::default(),
    )
    .shutdown_on_signal()
    .run(reconcile, error_policy, Arc::new(client))
    .for_each(|res| async move {
        if let Err(e) = res {
            tracing::error!(error = %e, "clusterconnection controller error");
        }
    })
    .await;
}

async fn reconcile(conn: Arc<ClusterConnection>, client: Arc<Client>) -> ReconcileResult {
    let ns = conn
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());
    let secret_ns = conn
        .spec
        .credentials_secret_ref
        .namespace
        .clone()
        .unwrap_or_else(|| ns.clone());
    let secret_name = &conn.spec.credentials_secret_ref.name;

    let secrets: Api<Secret> = Api::namespaced(client.as_ref().clone(), &secret_ns);
    let phase = match secrets.get(secret_name).await {
        Ok(_) => "Connected",
        Err(kube::Error::Api(e)) if e.code == 404 => "SecretMissing",
        Err(e) => {
            return Err(ReconcileError::Kube(e));
        }
    };

    let api: Api<ClusterConnection> = Api::namespaced(client.as_ref().clone(), &ns);
    if let Some(name) = &conn.metadata.name {
        let status = serde_json::json!({
            "status": {
                "phase": phase,
                "lastSyncTime": chrono::Utc::now().to_rfc3339(),
            }
        });
        api.patch_status(name, &Default::default(), &Patch::Merge(status))
            .await
            .map_err(ReconcileError::Kube)?;
    }
    Ok(Action::await_change())
}
