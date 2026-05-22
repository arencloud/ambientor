#![deny(unsafe_code)]

mod controllers;

use std::sync::Arc;

use ambientor_k8s::K8sClient;
use ambientor_rollout::RolloutEngine;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ambientor_operator=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let k8s = K8sClient::in_cluster().await?;
    let client = k8s.client.clone();

    if let Ok(url) = std::env::var("DATABASE_URL") {
        let pool = ambientor_db::connect(&url).await?;
        ambientor_db::migrate(&pool).await?;
        tracing::info!("database migrations applied");
    }

    let rollout_engine = Arc::new(RolloutEngine::new(client.clone()));

    controllers::run_all(client, rollout_engine).await;
    Ok(())
}
