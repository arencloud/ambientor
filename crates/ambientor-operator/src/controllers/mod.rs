pub mod assessment;
pub mod cluster;
pub mod context;
pub mod hub;
pub mod inventory;
pub mod migration_plan;
pub mod policy_translation;
pub mod rollout;
mod runtime;

use std::sync::Arc;

use ambientor_db::{AuditRepository, ScanRepository};
use ambientor_rollout::RolloutEngine;
use kube::Client;
use tracing::info;

pub use context::OperatorContext;

pub async fn run_all(
    client: Client,
    rollout_engine: Arc<RolloutEngine>,
    scan_repo: Option<Arc<ScanRepository>>,
    audit_repo: Option<Arc<AuditRepository>>,
) {
    info!("starting ambientor-operator controllers (kube-runtime watches)");
    let op_ctx = OperatorContext::new(client.clone(), rollout_engine, audit_repo);
    tokio::join!(
        inventory::run(client.clone()),
        assessment::run(client.clone(), scan_repo),
        migration_plan::run(client.clone()),
        policy_translation::run(client.clone()),
        rollout::run(op_ctx),
        cluster::run(client.clone()),
        hub::run(client),
    );
}
