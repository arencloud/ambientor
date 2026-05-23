pub mod assess;
pub mod assessments;
pub mod auth;
pub mod health;
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
        .route("/api/v1/events/{id}", get(sse::subscribe))
}
