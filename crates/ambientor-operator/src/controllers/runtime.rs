use std::time::Duration;

use kube::runtime::controller::Action;
use thiserror::Error;

/// Reconcile error type required by kube-runtime (`std::error::Error`).
#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error(transparent)]
    Kube(#[from] kube::Error),
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

pub type ReconcileResult = Result<Action, ReconcileError>;

/// Default error-policy requeue (transient API / scan failures).
pub(crate) fn error_policy<K, C>(
    _obj: std::sync::Arc<K>,
    err: &ReconcileError,
    _ctx: std::sync::Arc<C>,
) -> Action {
    tracing::warn!(error = %err, "reconcile failed, requeueing");
    Action::requeue(Duration::from_secs(30))
}
