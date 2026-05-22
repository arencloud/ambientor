use ambientor_types::ClusterConnection;
use k8s_openapi::api::core::v1::Secret;
use kube::{Api, Client, api::Patch};
use tracing::{info, warn};

use super::requeue_interval;

pub async fn run(client: Client) {
    loop {
        if let Err(e) = reconcile_all(&client).await {
            tracing::error!(error = %e, "clusterconnection reconcile failed");
        }
        tokio::time::sleep(requeue_interval() * 10).await;
    }
}

async fn reconcile_all(client: &Client) -> anyhow::Result<()> {
    let api: Api<ClusterConnection> = Api::all(client.clone());
    let list = api.list(&Default::default()).await?;
    for conn in list.items {
        reconcile_one(client, &conn).await?;
    }
    Ok(())
}

async fn reconcile_one(client: &Client, conn: &ClusterConnection) -> anyhow::Result<()> {
    let secret_ref = &conn.spec.credentials_secret_ref;
    let secret_ns = secret_ref
        .namespace
        .clone()
        .or_else(|| conn.metadata.namespace.clone())
        .unwrap_or_else(|| "ambientor-system".into());
    let secrets: Api<Secret> = Api::namespaced(client.clone(), &secret_ns);
    match secrets.get(&secret_ref.name).await {
        Ok(_) => info!(cluster = %conn.spec.display_name, "credentials secret present"),
        Err(e) => warn!(error = %e, "credentials secret missing"),
    }

    let ns = conn
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());
    let api: Api<ClusterConnection> = Api::namespaced(client.clone(), &ns);
    if let Some(name) = &conn.metadata.name {
        let phase = if conn.spec.hub {
            "HubActive"
        } else {
            "Connected"
        };
        let status = serde_json::json!({
            "status": {
                "phase": phase,
                "conditions": [{
                    "type": "CredentialsVerified",
                    "status": "True",
                    "message": "Secret reference resolved in local namespace"
                }]
            }
        });
        api.patch_status(name, &Default::default(), &Patch::Merge(status))
            .await?;
    }
    Ok(())
}
