use std::sync::Arc;

use ambientor_k8s::{RemoteClientError, client_for_connection, verify_connectivity};
use ambientor_types::ClusterConnection;
use chrono::Utc;
use futures::StreamExt;
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
            tracing::error!(error = ?e, "clusterconnection controller error");
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

    let (phase, ready, message) = match client_for_connection(client.as_ref(), &conn).await {
        Err(RemoteClientError::Api(kube::Error::Api(e))) if e.code == 404 => (
            "SecretMissing",
            "False",
            "credentials secret not found".to_string(),
        ),
        Err(e) => ("InvalidConfig", "False", e.to_string()),
        Ok(remote) => match verify_connectivity(&remote.client).await {
            Ok(version) => (
                "Connected",
                "True",
                format!("reachable; kubernetes {version}"),
            ),
            Err(RemoteClientError::Api(e)) => {
                ("Unreachable", "False", format!("API unreachable: {e}"))
            }
            Err(e) => ("InvalidConfig", "False", e.to_string()),
        },
    };

    let api: Api<ClusterConnection> = Api::namespaced(client.as_ref().clone(), &ns);
    if let Some(name) = &conn.metadata.name {
        let status = serde_json::json!({
            "status": {
                "phase": phase,
                "lastSyncTime": Utc::now().to_rfc3339(),
                "conditions": [{
                    "type": "Ready",
                    "status": ready,
                    "reason": phase,
                    "message": message,
                }],
            }
        });
        api.patch_status(name, &Default::default(), &Patch::Merge(status))
            .await
            .map_err(ReconcileError::Kube)?;
    }
    Ok(Action::await_change())
}
