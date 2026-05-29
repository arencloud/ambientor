//! Storage traits for scans, audit events, and users.

use std::sync::Arc;

use ambientor_types::dto::AuditEvent;
use async_trait::async_trait;
use uuid::Uuid;

use ambientor_dashboard::{
    ApplicationDetail, ApplicationListPage, ClusterAssessmentRun, DashboardResponse,
    FleetDashboardResponse,
};
use crate::applications::ApplicationListQuery;

use crate::pool::DbError;
use crate::repository::UserRecord;
use crate::scan::{ScanRunRow, StoredAssessment};

#[async_trait]
pub trait ScanStore: Send + Sync {
    async fn record_completed(
        &self,
        cluster_ref: &str,
        namespace: Option<&str>,
        payload: &StoredAssessment,
    ) -> Result<Uuid, DbError>;

    async fn list_recent(&self, limit: i64) -> Result<Vec<ScanRunRow>, DbError>;
}

#[async_trait]
pub trait AuditStore: Send + Sync {
    async fn append(&self, event: &AuditEvent) -> Result<(), DbError>;

    async fn list_by_resource(
        &self,
        resource: &str,
        limit: i64,
    ) -> Result<Vec<AuditEvent>, DbError>;

    async fn list_recent(&self, limit: i64) -> Result<Vec<AuditEvent>, DbError>;
}

#[async_trait]
pub trait ApplicationAssessmentStore: Send + Sync {
    async fn replace_run(&self, run: &ClusterAssessmentRun) -> Result<uuid::Uuid, DbError>;

    async fn list_applications(
        &self,
        query: ApplicationListQuery,
    ) -> Result<ApplicationListPage, DbError>;

    async fn get_application(
        &self,
        cluster_ref: &str,
        namespace: &str,
    ) -> Result<Option<ApplicationDetail>, DbError>;
}

#[async_trait]
pub trait DashboardStore: Send + Sync {
    async fn sync_snapshot(&self, response: &DashboardResponse) -> Result<(), DbError>;

    async fn load_by_cluster_ref(
        &self,
        cluster_ref: &str,
    ) -> Result<Option<DashboardResponse>, DbError>;

    async fn load_fleet(&self) -> Result<Option<FleetDashboardResponse>, DbError>;

    async fn is_snapshot_stale(&self, cluster_ref: &str) -> Result<bool, DbError>;

    async fn rebuild_from_latest_assessment(
        &self,
        cluster_ref: &str,
    ) -> Result<Option<DashboardResponse>, DbError>;
}

#[async_trait]
pub trait UserStore: Send + Sync {
    async fn find_by_username(&self, username: &str) -> Result<Option<UserRecord>, DbError>;

    async fn find_or_create_oidc(
        &self,
        username: &str,
        password_hash: &str,
        roles: &[String],
    ) -> Result<Uuid, DbError>;

    async fn create(
        &self,
        username: &str,
        password_hash: &str,
        roles: &[String],
    ) -> Result<Uuid, DbError>;
}

/// Opened database with pluggable store handles (Postgres today).
pub struct DbBackend {
    pub pool: sqlx::PgPool,
    pub scan: Arc<dyn ScanStore>,
    pub audit: Arc<dyn AuditStore>,
    pub dashboard: Arc<dyn DashboardStore>,
    pub applications: Arc<dyn ApplicationAssessmentStore>,
    pub users: Arc<dyn UserStore>,
}
