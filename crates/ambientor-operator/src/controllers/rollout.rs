use std::sync::Arc;

use ambientor_rollout::RolloutEngine;
use ambientor_types::Rollout;
use kube::{Api, Client, api::Patch};

use super::requeue_interval;

pub async fn run(client: Client, engine: Arc<RolloutEngine>) {
    loop {
        if let Err(e) = reconcile_all(&client, &engine).await {
            tracing::error!(error = %e, "rollout reconcile failed");
        }
        tokio::time::sleep(requeue_interval()).await;
    }
}

async fn reconcile_all(client: &Client, engine: &RolloutEngine) -> anyhow::Result<()> {
    let api: Api<Rollout> = Api::all(client.clone());
    let list = api.list(&Default::default()).await?;
    for obj in list.items {
        let phase = obj.status.as_ref().map(|s| s.phase.as_str()).unwrap_or("");
        if phase == "Completed" || phase == "Failed" {
            continue;
        }
        reconcile_one(client, engine, &obj).await?;
    }
    Ok(())
}

async fn reconcile_one(
    client: &Client,
    engine: &RolloutEngine,
    obj: &Rollout,
) -> anyhow::Result<()> {
    let mut status = obj.status.clone().unwrap_or_default();
    let _events = engine.reconcile(&obj.spec, &mut status).await?;

    let ns = obj
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());
    let api: Api<Rollout> = Api::namespaced(client.clone(), &ns);
    if let Some(name) = &obj.metadata.name {
        let patch = serde_json::json!({ "status": status });
        api.patch_status(name, &Default::default(), &Patch::Merge(patch))
            .await?;
    }
    Ok(())
}
