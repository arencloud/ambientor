use std::sync::Arc;

use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};

use crate::state::AppState;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Serialize)]
pub struct LoginResponse {
    pub token: String,
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    #[serde(default)]
    pub roles: Vec<String>,
}

pub async fn login(
    State(state): State<Arc<AppState>>,
    Json(body): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, (axum::http::StatusCode, String)> {
    let auth = state.auth.as_ref().ok_or((
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "database not configured".into(),
    ))?;
    let token = auth
        .login(&body.username, &body.password)
        .await
        .map_err(|e| (axum::http::StatusCode::UNAUTHORIZED, e.to_string()))?;
    Ok(Json(LoginResponse { token }))
}

pub async fn register(
    State(state): State<Arc<AppState>>,
    Json(body): Json<RegisterRequest>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let auth = state.auth.as_ref().ok_or((
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "database not configured".into(),
    ))?;
    let roles = if body.roles.is_empty() {
        vec!["viewer".into()]
    } else {
        body.roles
    };
    let id = auth
        .register_local(&body.username, &body.password, roles)
        .await
        .map_err(|e| (axum::http::StatusCode::BAD_REQUEST, e.to_string()))?;
    Ok(Json(serde_json::json!({ "id": id })))
}
