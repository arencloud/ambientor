pub mod assessment;
pub mod cluster;
pub mod context;
pub mod dashboard;
pub mod hub;
pub mod inventory;
pub mod migration_plan;
pub mod policy_translation;
pub mod rollout;
mod runtime;

use std::sync::Arc;

use ambientor_db::{ApplicationAssessmentStore, AuditStore, DashboardStore, ScanStore};
use ambientor_k8s::ClusterResourceCache;
use ambientor_rollout::RolloutEngine;
use kube::Client;
use tracing::info;

pub use context::OperatorContext;

pub async fn run_all(
    client: Client,
    rollout_engine: Arc<RolloutEngine>,
    scan_repo: Option<Arc<dyn ScanStore>>,
    audit_repo: Option<Arc<dyn AuditStore>>,
    dashboard_repo: Option<Arc<dyn DashboardStore>>,
    applications_repo: Option<Arc<dyn ApplicationAssessmentStore>>,
    resource_cache: Arc<ClusterResourceCache>,
) {
    info!("starting ambientor-operator controllers (kube-runtime watches)");
    let op_ctx = OperatorContext::new(client.clone(), rollout_engine, audit_repo);
    let dashboard_client = client.clone();
    let dashboard_store = dashboard_repo.clone();
    let scan_for_plans = scan_repo.clone();
    let scan_for_dashboard = scan_repo.clone();
    tokio::join!(
        inventory::run(client.clone()),
        assessment::run(
            client.clone(),
            scan_repo,
            dashboard_repo,
            applications_repo,
            Some(resource_cache.clone()),
        ),
        migration_plan::run(client.clone(), scan_for_plans),
        policy_translation::run(client.clone()),
        rollout::run(op_ctx),
        cluster::run(client.clone()),
        hub::run(client.clone()),
        dashboard::run(dashboard_client, dashboard_store, scan_for_dashboard),
    );
}
