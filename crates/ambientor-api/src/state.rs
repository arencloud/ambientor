use std::sync::Arc;

use ambientor_auth::jwt::JwtService;
use ambientor_auth::rbac::RbacEnforcer;
use ambientor_auth::service::AuthService;
use ambientor_db::{AuditRepository, ScanRepository, UserRepository};
use sqlx::PgPool;
use tokio::sync::RwLock;

use crate::routes::sse::SseHub;

pub struct AppState {
    pub auth: Option<Arc<AuthService>>,
    pub sse: Arc<RwLock<SseHub>>,
    pool: Option<PgPool>,
    #[allow(dead_code)]
    jwt: JwtService,
}

impl AppState {
    pub async fn from_env() -> anyhow::Result<Self> {
        let secret = std::env::var("AMBIENTOR_JWT_SECRET")
            .unwrap_or_else(|_| "dev-secret-change-in-production".into());
        let jwt = JwtService::new(secret.as_bytes());
        let sse = Arc::new(RwLock::new(SseHub::new()));

        let pool = match std::env::var("DATABASE_URL") {
            Ok(url) => {
                let pool = ambientor_db::connect(&url).await?;
                ambientor_db::migrate(&pool).await?;
                Some(pool)
            }
            Err(_) => {
                tracing::warn!("DATABASE_URL not set; running without persistence");
                None
            }
        };

        let auth = if let Some(ref pool) = pool {
            let users = UserRepository::new(pool.clone());
            let rbac = RbacEnforcer::with_defaults().await?;
            Some(Arc::new(AuthService {
                users,
                jwt: JwtService::new(secret.as_bytes()),
                rbac,
            }))
        } else {
            None
        };

        Ok(Self {
            pool,
            auth,
            jwt,
            sse,
        })
    }

    #[allow(dead_code)]
    pub fn audit_repo(&self) -> Option<AuditRepository> {
        self.pool.as_ref().map(|p| AuditRepository::new(p.clone()))
    }

    pub fn scan_repo(&self) -> Option<ScanRepository> {
        self.pool.as_ref().map(|p| ScanRepository::new(p.clone()))
    }
}
