use ambientor_types::dto::AuditEvent;
use async_trait::async_trait;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

use crate::pool::DbError;
use crate::traits::{AuditStore, UserStore};

#[derive(Clone, FromRow)]
pub struct UserRecord {
    pub id: Uuid,
    pub username: String,
    pub password_hash: String,
    pub roles: Vec<String>,
}

pub struct UserRepository {
    pool: PgPool,
}

impl UserRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn find_by_username(&self, username: &str) -> Result<Option<UserRecord>, DbError> {
        let row = sqlx::query_as::<_, UserRecord>(
            "SELECT id, username, password_hash, roles FROM users WHERE username = $1 AND disabled = false",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn find_or_create_oidc(
        &self,
        username: &str,
        password_hash: &str,
        roles: &[String],
    ) -> Result<Uuid, DbError> {
        if let Some(user) = self.find_by_username(username).await? {
            return Ok(user.id);
        }
        self.create(username, password_hash, roles).await
    }

    pub async fn create(
        &self,
        username: &str,
        password_hash: &str,
        roles: &[String],
    ) -> Result<Uuid, DbError> {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, roles) VALUES ($1, $2, $3, $4)",
        )
        .bind(id)
        .bind(username)
        .bind(password_hash)
        .bind(roles)
        .execute(&self.pool)
        .await?;
        Ok(id)
    }
}

pub struct AuditRepository {
    pool: PgPool,
}

impl AuditRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn append(&self, event: &AuditEvent) -> Result<(), DbError> {
        sqlx::query(
            "INSERT INTO audit_events (id, timestamp, actor, action, resource, outcome, details) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        )
        .bind(event.id)
        .bind(event.timestamp)
        .bind(&event.actor)
        .bind(&event.action)
        .bind(&event.resource)
        .bind(&event.outcome)
        .bind(&event.details)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_by_resource(
        &self,
        resource: &str,
        limit: i64,
    ) -> Result<Vec<AuditEvent>, DbError> {
        let rows = sqlx::query_as::<_, AuditRow>(
            "SELECT id, timestamp, actor, action, resource, outcome, details FROM audit_events WHERE resource = $1 ORDER BY timestamp DESC LIMIT $2",
        )
        .bind(resource)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(audit_row_to_event).collect())
    }

    pub async fn list_recent(&self, limit: i64) -> Result<Vec<AuditEvent>, DbError> {
        let rows = sqlx::query_as::<_, AuditRow>(
            "SELECT id, timestamp, actor, action, resource, outcome, details FROM audit_events ORDER BY timestamp DESC LIMIT $1",
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(audit_row_to_event).collect())
    }
}

#[async_trait]
impl UserStore for UserRepository {
    async fn find_by_username(&self, username: &str) -> Result<Option<UserRecord>, DbError> {
        UserRepository::find_by_username(self, username).await
    }

    async fn find_or_create_oidc(
        &self,
        username: &str,
        password_hash: &str,
        roles: &[String],
    ) -> Result<Uuid, DbError> {
        UserRepository::find_or_create_oidc(self, username, password_hash, roles).await
    }

    async fn create(
        &self,
        username: &str,
        password_hash: &str,
        roles: &[String],
    ) -> Result<Uuid, DbError> {
        UserRepository::create(self, username, password_hash, roles).await
    }
}

#[async_trait]
impl AuditStore for AuditRepository {
    async fn append(&self, event: &AuditEvent) -> Result<(), DbError> {
        AuditRepository::append(self, event).await
    }

    async fn list_by_resource(
        &self,
        resource: &str,
        limit: i64,
    ) -> Result<Vec<AuditEvent>, DbError> {
        AuditRepository::list_by_resource(self, resource, limit).await
    }

    async fn list_recent(&self, limit: i64) -> Result<Vec<AuditEvent>, DbError> {
        AuditRepository::list_recent(self, limit).await
    }
}

fn audit_row_to_event(r: AuditRow) -> AuditEvent {
    AuditEvent {
        id: r.id,
        timestamp: r.timestamp,
        actor: r.actor,
        action: r.action,
        resource: r.resource,
        outcome: r.outcome,
        details: r.details,
    }
}

#[derive(FromRow)]
struct AuditRow {
    id: Uuid,
    timestamp: chrono::DateTime<chrono::Utc>,
    actor: String,
    action: String,
    resource: String,
    outcome: String,
    details: Option<serde_json::Value>,
}
