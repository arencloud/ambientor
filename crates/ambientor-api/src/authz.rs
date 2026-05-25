use std::sync::Arc;

use ambientor_auth::jwt::Claims;
use ambientor_auth::rbac::object_in_namespace;
use axum::http::{HeaderMap, StatusCode, header::AUTHORIZATION};

use crate::state::AppState;

pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
}

pub fn require_claims(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Claims, (StatusCode, String)> {
    let token = bearer_token(headers).ok_or((
        StatusCode::UNAUTHORIZED,
        "missing Authorization: Bearer token".into(),
    ))?;
    state
        .verify_jwt(token)
        .map_err(|_| (StatusCode::UNAUTHORIZED, "invalid or expired token".into()))
}

pub async fn require_rollout_approve(
    state: &Arc<AppState>,
    headers: &HeaderMap,
    namespace: &str,
    rollout_name: &str,
) -> Result<Claims, (StatusCode, String)> {
    let claims = require_claims(state, headers)?;
    let auth = state.auth.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "auth not configured".into(),
    ))?;
    let object = object_in_namespace(namespace, "rollout", rollout_name);
    let allowed = auth
        .authorize(&claims, namespace, &object, "approve")
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if !allowed {
        return Err((StatusCode::FORBIDDEN, "insufficient permissions".into()));
    }
    Ok(claims)
}
