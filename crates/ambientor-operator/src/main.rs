#![deny(unsafe_code)]

mod controllers;

use std::sync::Arc;

use ambientor_k8s::{ClusterResourceCache, K8sClient};
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

    let (scan_repo, audit_repo, dashboard_repo, applications_repo) =
        if let Ok(url) = std::env::var("DATABASE_URL") {
            let db = ambientor_db::open_postgres(&url).await?;
            tracing::info!("database migrations applied");
            (
                Some(db.scan),
                Some(db.audit),
                Some(db.dashboard),
                Some(db.applications),
            )
        } else {
            tracing::warn!("DATABASE_URL not set; scans and audit log will not be persisted");
            (None, None, None, None)
        };

    let rollout_engine = Arc::new(RolloutEngine::new(client.clone()));
    let resource_cache = Arc::new(ClusterResourceCache::spawn(client.clone()));

    controllers::run_all(
        client,
        rollout_engine,
        scan_repo,
        audit_repo,
        dashboard_repo,
        applications_repo,
        resource_cache,
    )
    .await;
    Ok(())
}
