pub mod assessment;
pub mod cluster;
pub mod context;
pub mod hub;
pub mod inventory;
pub mod rollout;
mod runtime;

use std::sync::Arc;

use ambientor_db::ScanRepository;
use ambientor_rollout::RolloutEngine;
use kube::Client;
use tracing::info;

pub use context::OperatorContext;

pub async fn run_all(
    client: Client,
    rollout_engine: Arc<RolloutEngine>,
    scan_repo: Option<Arc<ScanRepository>>,
) {
    info!("starting ambientor-operator controllers (kube-runtime watches)");
    let op_ctx = OperatorContext::new(client.clone(), rollout_engine);
    tokio::join!(
        inventory::run(client.clone()),
        assessment::run(client.clone(), scan_repo),
        rollout::run(op_ctx),
        cluster::run(client.clone()),
        hub::run(client),
    );
}
