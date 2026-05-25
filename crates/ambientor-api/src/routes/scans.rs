use std::sync::Arc;

use ambientor_db::StoredAssessment;
use axum::{
    Json,
    extract::{Query, State},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::state::AppState;

#[derive(Deserialize)]
pub struct ListScansQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanListItem {
    pub id: Uuid,
    pub cluster_ref: Option<String>,
    pub namespace: Option<String>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
    pub status: String,
    pub assessment: StoredAssessment,
}

pub async fn list_scans(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListScansQuery>,
) -> Result<Json<Vec<ScanListItem>>, (axum::http::StatusCode, String)> {
    let repo = state.scan_store().ok_or((
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "DATABASE_URL not configured".into(),
    ))?;

    let limit = query.limit.clamp(1, 200);
    let rows = repo
        .list_recent(limit)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let assessment: StoredAssessment = serde_json::from_value(row.assessment_json)
            .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        items.push(ScanListItem {
            id: row.id,
            cluster_ref: row.cluster_ref,
            namespace: row.namespace,
            finished_at: row.finished_at,
            status: row.status,
            assessment,
        });
    }
    Ok(Json(items))
}
