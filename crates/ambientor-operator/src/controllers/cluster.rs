use ambientor_k8s::detect_platform;
use ambientor_types::Cluster;
use kube::{Api, Client, api::Patch};

use super::requeue_interval;

pub async fn run(client: Client) {
    loop {
        if let Err(e) = reconcile_all(&client).await {
            tracing::error!(error = %e, "cluster reconcile failed");
        }
        tokio::time::sleep(requeue_interval() * 4).await;
    }
}

async fn reconcile_all(client: &Client) -> anyhow::Result<()> {
    let api: Api<Cluster> = Api::all(client.clone());
    let list = api.list(&Default::default()).await?;
    let platform = detect_platform(client).await.unwrap_or_default();
    for obj in list.items {
        if let Some(name) = &obj.metadata.name {
            let status = serde_json::json!({
                "status": {
                    "phase": "Ready",
                    "meshVersion": platform.version,
                }
            });
            api.patch_status(name, &Default::default(), &Patch::Merge(status))
                .await?;
        }
    }
    Ok(())
}
