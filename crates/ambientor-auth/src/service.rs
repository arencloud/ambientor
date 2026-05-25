use std::sync::Arc;

use ambientor_db::UserStore;
use ambientor_types::dto::AuditEvent;
use chrono::Utc;
use uuid::Uuid;

use crate::jwt::{Claims, JwtService};
use crate::password::{PasswordError, hash_password, verify_password};
use crate::rbac::RbacEnforcer;

pub struct AuthService {
    pub users: Arc<dyn UserStore>,
    pub jwt: JwtService,
    pub rbac: RbacEnforcer,
}

impl AuthService {
    pub async fn register_local(
        &self,
        username: &str,
        password: &str,
        roles: Vec<String>,
    ) -> Result<Uuid, PasswordError> {
        let hash = hash_password(password)?;
        self.users
            .create(username, &hash, &roles)
            .await
            .map_err(|e| PasswordError::Hash(e.to_string()))
    }

    pub async fn login_oidc(
        &self,
        identity: &crate::oidc_flow::OidcIdentity,
        default_roles: &[String],
    ) -> Result<String, AuthError> {
        let db_username = format!("oidc:{}", identity.subject);
        let placeholder_hash = hash_password(&uuid::Uuid::new_v4().to_string())
            .map_err(|e| AuthError::Db(e.to_string()))?;
        let roles = if default_roles.is_empty() {
            vec!["viewer".to_string()]
        } else {
            default_roles.to_vec()
        };
        let id = self
            .users
            .find_or_create_oidc(&db_username, &placeholder_hash, &roles)
            .await
            .map_err(|e| AuthError::Db(e.to_string()))?;
        self.jwt
            .issue(id, &identity.username, roles)
            .map_err(|e| AuthError::Token(e.to_string()))
    }

    pub async fn login(&self, username: &str, password: &str) -> Result<String, AuthError> {
        let user = self
            .users
            .find_by_username(username)
            .await
            .map_err(|e| AuthError::Db(e.to_string()))?
            .ok_or(AuthError::InvalidCredentials)?;
        verify_password(password, &user.password_hash)
            .map_err(|_| AuthError::InvalidCredentials)?;
        self.jwt
            .issue(user.id, &user.username, user.roles)
            .map_err(|e| AuthError::Token(e.to_string()))
    }

    pub async fn authorize(
        &self,
        claims: &Claims,
        namespace: &str,
        object: &str,
        action: &str,
    ) -> Result<bool, AuthError> {
        for role in &claims.roles {
            if self
                .rbac
                .enforce(role, namespace, object, action)
                .map_err(|e| AuthError::Rbac(e.to_string()))?
            {
                return Ok(true);
            }
        }
        if self
            .rbac
            .enforce(&claims.username, namespace, object, action)
            .map_err(|e| AuthError::Rbac(e.to_string()))?
        {
            return Ok(true);
        }
        Ok(false)
    }
}

#[derive(Debug)]
pub enum AuthError {
    InvalidCredentials,
    Db(String),
    Token(String),
    Rbac(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidCredentials => write!(f, "invalid credentials"),
            Self::Db(e) => write!(f, "database error: {e}"),
            Self::Token(e) => write!(f, "token error: {e}"),
            Self::Rbac(e) => write!(f, "rbac error: {e}"),
        }
    }
}

impl std::error::Error for AuthError {}

pub fn audit(actor: &str, action: &str, resource: &str, outcome: &str) -> AuditEvent {
    AuditEvent {
        id: Uuid::new_v4(),
        timestamp: Utc::now(),
        actor: actor.to_string(),
        action: action.to_string(),
        resource: resource.to_string(),
        outcome: outcome.to_string(),
        details: None,
    }
}
