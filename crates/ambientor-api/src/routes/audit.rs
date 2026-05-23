use std::sync::Arc;

use ambientor_rollout::audit::rollout_resource;
use ambientor_types::AuditEvent;
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct AuditListQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    pub resource: Option<String>,
}

fn default_limit() -> i64 {
    100
}

pub async fn list_audit(
    State(state): State<Arc<AppState>>,
    Query(query): Query<AuditListQuery>,
) -> Result<Json<Vec<AuditEvent>>, (StatusCode, String)> {
    let repo = state.audit_repo().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "DATABASE_URL not configured; audit log unavailable".into(),
    ))?;
    let limit = query.limit.clamp(1, 500);
    let events = if let Some(resource) = query.resource {
        repo.list_by_resource(&resource, limit)
            .await
            .map_err(internal)?
    } else {
        repo.list_recent(limit).await.map_err(internal)?
    };
    Ok(Json(events))
}

pub async fn list_rollout_audit(
    State(state): State<Arc<AppState>>,
    Path((namespace, name)): Path<(String, String)>,
    Query(query): Query<AuditListQuery>,
) -> Result<Json<Vec<AuditEvent>>, (StatusCode, String)> {
    let resource = rollout_resource(&namespace, &name);
    let repo = state.audit_repo().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "DATABASE_URL not configured; audit log unavailable".into(),
    ))?;
    let limit = query.limit.clamp(1, 500);
    let events = repo
        .list_by_resource(&resource, limit)
        .await
        .map_err(internal)?;
    Ok(Json(events))
}

fn internal<E: std::fmt::Display>(e: E) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
