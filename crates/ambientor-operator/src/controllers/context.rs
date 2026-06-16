use std::sync::Arc;

use ambientor_db::{AuditStore, DashboardStore, ScanStore};
use ambientor_rollout::RolloutEngine;
use kube::Client;

/// Shared context passed to kube-runtime controllers.
pub struct OperatorContext {
    pub client: Client,
    pub rollout_engine: Arc<RolloutEngine>,
    pub audit_repo: Option<Arc<dyn AuditStore>>,
    pub dashboard_repo: Option<Arc<dyn DashboardStore>>,
    pub scan_repo: Option<Arc<dyn ScanStore>>,
}

impl OperatorContext {
    pub fn new(
        client: Client,
        rollout_engine: Arc<RolloutEngine>,
        audit_repo: Option<Arc<dyn AuditStore>>,
        dashboard_repo: Option<Arc<dyn DashboardStore>>,
        scan_repo: Option<Arc<dyn ScanStore>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            client,
            rollout_engine,
            audit_repo,
            dashboard_repo,
            scan_repo,
        })
    }
}
