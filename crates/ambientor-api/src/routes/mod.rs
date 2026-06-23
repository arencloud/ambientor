pub mod applications;
pub mod assess;
pub mod assessment_crd;
pub mod assessments;
pub mod audit;
pub mod auth;
pub mod connections;
pub mod dashboard;
pub mod health;
pub mod mesh_instances;
pub mod openshift;
pub mod plans;
pub mod rollout_watch;
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
        .route("/api/v1/auth/config", get(auth::auth_config))
        .route("/api/v1/auth/login", post(auth::login))
        .route("/api/v1/auth/register", post(auth::register))
        .route("/api/v1/auth/oidc/login", get(auth::oidc_login))
        .route("/api/v1/auth/oidc/callback", get(auth::oidc_callback))
        .route("/api/v1/assess", post(assess::assess))
        .route(
            "/api/v1/connections",
            get(connections::list_connections).post(connections::create_connection),
        )
        .route(
            "/api/v1/connections/{namespace}/{name}",
            get(connections::get_connection).delete(connections::delete_connection),
        )
        .route(
            "/api/v1/connections/{namespace}/{name}/export",
            get(connections::export_connection),
        )
        .route(
            "/api/v1/connections/{namespace}/{name}/assess",
            post(connections::assess_connection),
        )
        .route("/api/v1/dashboard", get(dashboard::get_dashboard))
        .route(
            "/api/v1/dashboard/fleet",
            get(dashboard::get_fleet_dashboard),
        )
        .route("/api/v1/applications", get(applications::list_applications))
        .route(
            "/api/v1/applications/{namespace}",
            get(applications::get_application),
        )
        .route("/api/v1/assessments", get(assessments::list_assessments))
        .route(
            "/api/v1/mesh-instances",
            get(mesh_instances::list_mesh_instances),
        )
        .route(
            "/api/v1/mesh-instances/enroll",
            post(mesh_instances::enroll_namespace),
        )
        .route("/api/v1/scans", get(scans::list_scans))
        .route(
            "/api/v1/plans",
            get(plans::list_plans).post(plans::create_plan),
        )
        .route("/api/v1/plans/{namespace}/{name}", get(plans::get_plan))
        .route(
            "/api/v1/plans/{namespace}/{name}/export",
            get(plans::export_plan),
        )
        .route(
            "/api/v1/plans/{namespace}/{name}/approve",
            post(plans::approve_plan),
        )
        .route(
            "/api/v1/plans/{namespace}/{name}/execute",
            post(plans::execute_plan),
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
        .route(
            "/api/v1/rollouts/{namespace}/{name}/audit",
            get(audit::list_rollout_audit),
        )
        .route("/api/v1/audit", get(audit::list_audit))
        .route("/api/v1/openshift/wizard", get(openshift::openshift_wizard))
        .route("/api/v1/events/{id}", get(sse::subscribe))
}
