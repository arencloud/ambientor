use std::sync::Arc;

use ambientor_rollout::RolloutEngine;
use kube::Client;

/// Shared context passed to kube-runtime controllers.
pub struct OperatorContext {
    pub client: Client,
    pub rollout_engine: Arc<RolloutEngine>,
}

impl OperatorContext {
    pub fn new(client: Client, rollout_engine: Arc<RolloutEngine>) -> Arc<Self> {
        Arc::new(Self {
            client,
            rollout_engine,
        })
    }
}
