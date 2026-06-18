use std::sync::Arc;
use std::time::Duration;

use ambientor_k8s::{RemoteClientError, client_for_connection, rollout_access_gaps, verify_connectivity};
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

    let (phase, ready, message, rollout_access) = match client_for_connection(client.as_ref(), &conn).await {
        Err(RemoteClientError::Api(kube::Error::Api(e))) if e.code == 404 => (
            "SecretMissing",
            "False",
            "credentials secret not found".to_string(),
            None,
        ),
        Err(e) => ("InvalidConfig", "False", e.to_string(), None),
        Ok(remote) => match verify_connectivity(&remote.client).await {
            Ok(version) => {
                let gaps = rollout_access_gaps(&remote.client).await.unwrap_or_else(|e| {
                    vec![format!("RBAC check failed: {e}")]
                });
                let (rollout_status, rollout_msg) = if gaps.is_empty() {
                    ("True", "spoke credentials can run hub-orchestrated rollouts".to_string())
                } else {
                    (
                        "False",
                        format!(
                            "missing rollout permissions: {}. On the spoke: kubectl apply -f docs/lab/spoke-hub-remote-rbac.yaml",
                            gaps.join(", ")
                        ),
                    )
                };
                (
                    "Connected",
                    "True",
                    format!("reachable; kubernetes {version}"),
                    Some((rollout_status, rollout_msg)),
                )
            }
            Err(RemoteClientError::Api(e)) => {
                ("Unreachable", "False", format!("API unreachable: {e}"), None)
            }
            Err(e) => ("InvalidConfig", "False", e.to_string(), None),
        },
    };

    let mut conditions = vec![serde_json::json!({
        "type": "Ready",
        "status": ready,
        "reason": phase,
        "message": message,
    })];
    if let Some((rollout_status, rollout_msg)) = rollout_access {
        conditions.push(serde_json::json!({
            "type": "RolloutAccess",
            "status": rollout_status,
            "reason": if rollout_status == "True" { "SufficientRBAC" } else { "InsufficientRBAC" },
            "message": rollout_msg,
        }));
    }

    let api: Api<ClusterConnection> = Api::namespaced(client.as_ref().clone(), &ns);
    if let Some(name) = &conn.metadata.name {
        let status = serde_json::json!({
            "status": {
                "phase": phase,
                "lastSyncTime": Utc::now().to_rfc3339(),
                "conditions": conditions,
            }
        });
        api.patch_status(name, &Default::default(), &Patch::Merge(status))
            .await
            .map_err(ReconcileError::Kube)?;
    }
    Ok(if phase == "Connected" {
        Action::requeue(Duration::from_secs(120))
    } else {
        Action::await_change()
    })
}
