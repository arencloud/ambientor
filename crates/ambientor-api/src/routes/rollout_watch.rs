//! Push rollout status changes to SSE subscribers (live portal updates).

use std::sync::Arc;
use std::time::Duration;

use ambientor_k8s::K8sClient;
use ambientor_types::Rollout;
use futures::StreamExt;
use kube::{
    Api,
    runtime::watcher::{watcher, Config, Event},
};

use crate::state::AppState;

pub fn spawn_rollout_watch(state: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            match watch_rollouts(state.clone()).await {
                Ok(()) => tracing::info!("rollout watch stream ended; reconnecting"),
                Err(e) => tracing::warn!(error = %e, "rollout watch failed; retrying"),
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    });
}

async fn watch_rollouts(state: Arc<AppState>) -> anyhow::Result<()> {
    let k8s = hub_k8s_client().await?;
    let api: Api<Rollout> = Api::all(k8s.client);
    let mut stream = Box::pin(watcher(api, Config::default().any_semantic()));
    while let Some(event) = stream.next().await {
        let event = event?;
        match event {
            Event::Apply(r) | Event::InitApply(r) | Event::Delete(r) => {
                publish_rollout_event(&state, &r).await;
            }
            Event::Init | Event::InitDone => {}
        }
    }
    Ok(())
}

async fn hub_k8s_client() -> anyhow::Result<K8sClient> {
    K8sClient::in_cluster()
        .await
        .or(K8sClient::from_kubeconfig(None).await)
        .map_err(|e| anyhow::anyhow!("kubernetes client: {e}"))
}

pub async fn publish_rollout_event(state: &AppState, rollout: &Rollout) {
    let Some(name) = rollout.metadata.name.clone() else {
        return;
    };
    let namespace = rollout
        .metadata
        .namespace
        .clone()
        .unwrap_or_else(|| "default".into());
    let status = rollout.status.as_ref();
    let phase = status.map(|s| s.phase.as_str()).unwrap_or("Unknown");
    let current_stage = status.map(|s| s.current_stage).unwrap_or(0);
    let approved_stage = status.map(|s| s.approved_stage).unwrap_or(-1);
    state.sse.write().await.publish(
        "rollout",
        &serde_json::json!({
            "namespace": namespace,
            "name": name,
            "phase": phase,
            "currentStage": current_stage,
            "approvedStage": approved_stage,
            "clusterRef": rollout.spec.cluster_ref,
            "planRef": rollout.spec.plan_ref,
        }),
    );
}
