use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    response::{IntoResponse, Redirect},
};
use serde::{Deserialize, Serialize};
use url::Url;

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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthConfigResponse {
    pub enabled: bool,
    pub local_login: bool,
    pub oidc_login_url: Option<String>,
    pub require_auth_for_approve: bool,
}

/// Portal/auth clients discover whether login and OIDC are available.
pub async fn auth_config(State(state): State<Arc<AppState>>) -> Json<AuthConfigResponse> {
    let enabled = state.auth.is_some();
    Json(AuthConfigResponse {
        enabled,
        local_login: enabled,
        oidc_login_url: if state.oidc.is_some() {
            Some("/api/v1/auth/oidc/login".into())
        } else {
            None
        },
        require_auth_for_approve: enabled,
    })
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

/// Redirect browser to the IdP authorization endpoint (PKCE + CSRF state stored server-side).
pub async fn oidc_login(
    State(state): State<Arc<AppState>>,
) -> Result<Redirect, (axum::http::StatusCode, String)> {
    let oidc = state.oidc.as_ref().ok_or((
        axum::http::StatusCode::NOT_FOUND,
        "OIDC not configured".into(),
    ))?;
    let url = oidc
        .flow
        .authorize_url(&oidc.config)
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Redirect::temporary(url.as_str()))
}

#[derive(Deserialize)]
pub struct OidcCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// Exchange authorization code for JWT; redirect to portal or return JSON token.
pub async fn oidc_callback(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OidcCallbackQuery>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    if let Some(err) = query.error {
        let detail = query.error_description.unwrap_or_default();
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            format!("OIDC error: {err} {detail}").trim().to_string(),
        ));
    }
    let code = query
        .code
        .ok_or((axum::http::StatusCode::BAD_REQUEST, "missing code".into()))?;
    let csrf_state = query
        .state
        .ok_or((axum::http::StatusCode::BAD_REQUEST, "missing state".into()))?;

    let oidc = state.oidc.as_ref().ok_or((
        axum::http::StatusCode::NOT_FOUND,
        "OIDC not configured".into(),
    ))?;
    let auth = state.auth.as_ref().ok_or((
        axum::http::StatusCode::SERVICE_UNAVAILABLE,
        "database not configured".into(),
    ))?;

    let identity = oidc
        .flow
        .exchange_callback(&code, &csrf_state)
        .await
        .map_err(|e| (axum::http::StatusCode::UNAUTHORIZED, e.to_string()))?;
    let token = auth
        .login_oidc(&identity, &oidc.default_roles)
        .await
        .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if let Some(success) = &oidc.success_redirect {
        let mut url = Url::parse(success)
            .map_err(|e| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        url.query_pairs_mut().append_pair("token", &token);
        return Ok(Redirect::temporary(url.as_str()).into_response());
    }

    Ok(Json(LoginResponse { token }).into_response())
}
