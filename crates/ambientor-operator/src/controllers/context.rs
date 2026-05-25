use std::sync::Arc;

use ambientor_db::AuditStore;
use ambientor_rollout::RolloutEngine;
use kube::Client;

/// Shared context passed to kube-runtime controllers.
pub struct OperatorContext {
    pub client: Client,
    pub rollout_engine: Arc<RolloutEngine>,
    pub audit_repo: Option<Arc<dyn AuditStore>>,
}

impl OperatorContext {
    pub fn new(
        client: Client,
        rollout_engine: Arc<RolloutEngine>,
        audit_repo: Option<Arc<dyn AuditStore>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            client,
            rollout_engine,
            audit_repo,
        })
    }
}
