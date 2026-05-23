pub mod assess;
pub mod assessments;
pub mod auth;
pub mod health;
pub mod plans;
pub mod rollouts;
pub mod scans;
pub mod sse;

use std::sync::Arc;

use axum::Router;
use axum::routing::{get, post};

use crate::state::AppState;

pub fn api_router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/healthz", get(health::healthz))
        .route("/readyz", get(health::readyz))
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/register", post(auth::register))
        .route("/api/v1/assess", post(assess::assess))
        .route("/api/v1/assessments", get(assessments::list_assessments))
        .route("/api/v1/scans", get(scans::list_scans))
        .route("/api/v1/plans", get(plans::list_plans))
        .route("/api/v1/plans/{namespace}/{name}", get(plans::get_plan))
        .route(
            "/api/v1/plans/{namespace}/{name}/export",
            get(plans::export_plan),
        )
        .route(
            "/api/v1/plans/{namespace}/{name}/rollout",
            post(rollouts::create_rollout_from_plan),
        )
        .route("/api/v1/rollouts", get(rollouts::list_rollouts))
        .route(
            "/api/v1/rollouts/{namespace}/{name}",
            get(rollouts::get_rollout),
        )
        .route(
            "/api/v1/rollouts/{namespace}/{name}/approve",
            post(rollouts::approve_rollout),
        )
        .route("/api/v1/events/{id}", get(sse::subscribe))
}
