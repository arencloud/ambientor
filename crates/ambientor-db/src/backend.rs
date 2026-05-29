use std::sync::Arc;

use crate::pool::{self, DbError};
use crate::applications::ApplicationAssessmentRepository;
use crate::dashboard::DashboardRepository;
use crate::repository::{AuditRepository, UserRepository};
use crate::scan::ScanRepository;
use crate::traits::{
    ApplicationAssessmentStore, AuditStore, DashboardStore, DbBackend, ScanStore, UserStore,
};

/// Connect, migrate, and return store trait objects backed by Postgres.
pub async fn open_postgres(database_url: &str) -> Result<DbBackend, DbError> {
    let pool = pool::connect(database_url).await?;
    pool::migrate(&pool).await?;
    Ok(DbBackend {
        scan: Arc::new(ScanRepository::new(pool.clone())) as Arc<dyn ScanStore>,
        audit: Arc::new(AuditRepository::new(pool.clone())) as Arc<dyn AuditStore>,
        dashboard: Arc::new(DashboardRepository::new(pool.clone())) as Arc<dyn DashboardStore>,
        applications: Arc::new(ApplicationAssessmentRepository::new(pool.clone()))
            as Arc<dyn ApplicationAssessmentStore>,
        users: Arc::new(UserRepository::new(pool.clone())) as Arc<dyn UserStore>,
        pool,
    })
}
