pub mod assessment;
pub mod cluster;
pub mod hub;
pub mod inventory;
pub mod rollout;

use std::sync::Arc;
use std::time::Duration;

use ambientor_rollout::RolloutEngine;
use kube::Client;
use tracing::info;

pub async fn run_all(client: Client, rollout_engine: Arc<RolloutEngine>) {
    info!("starting ambientor-operator controllers");
    tokio::join!(
        inventory::run(client.clone()),
        assessment::run(client.clone()),
        rollout::run(client.clone(), rollout_engine),
        cluster::run(client.clone()),
        hub::run(client),
    );
}

pub(crate) fn requeue_interval() -> Duration {
    Duration::from_secs(30)
}
