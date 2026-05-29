use std::sync::Arc;

use ambientor_auth::jwt::{Claims, JwtService};
use ambientor_auth::oidc::{OidcConfig, oidc_config_from_env, oidc_default_roles_from_env};
use ambientor_auth::oidc_flow::OidcFlowService;
use ambientor_auth::rbac::RbacEnforcer;
use ambientor_auth::service::AuthService;
use ambientor_db::{
    ApplicationAssessmentStore, AuditStore, DashboardStore, DbBackend, ScanStore, open_postgres,
};
use tokio::sync::RwLock;

use crate::routes::sse::SseHub;

/// OIDC authorization-code flow (discovery at API startup).
pub struct OidcState {
    pub flow: Arc<OidcFlowService>,
    pub config: OidcConfig,
    pub default_roles: Vec<String>,
    /// Browser redirect after successful login (`?token=` appended).
    pub success_redirect: Option<String>,
}

pub struct AppState {
    pub auth: Option<Arc<AuthService>>,
    pub oidc: Option<OidcState>,
    pub sse: Arc<RwLock<SseHub>>,
    db: Option<DbBackend>,
    #[allow(dead_code)]
    jwt: JwtService,
}

impl AppState {
    pub async fn from_env() -> anyhow::Result<Self> {
        let secret = std::env::var("AMBIENTOR_JWT_SECRET")
            .unwrap_or_else(|_| "dev-secret-change-in-production".into());
        let jwt = JwtService::new(secret.as_bytes());
        let sse = Arc::new(RwLock::new(SseHub::new()));

        let db = match std::env::var("DATABASE_URL") {
            Ok(url) => Some(open_postgres(&url).await?),
            Err(_) => {
                tracing::warn!("DATABASE_URL not set; running without persistence");
                None
            }
        };

        let auth = if let Some(ref backend) = db {
            let rbac = RbacEnforcer::with_postgres(backend.pool.clone()).await?;
            Some(Arc::new(AuthService {
                users: backend.users.clone(),
                jwt: JwtService::new(secret.as_bytes()),
                rbac,
            }))
        } else {
            None
        };

        let oidc = if auth.is_some() {
            if let Some(config) = oidc_config_from_env() {
                match OidcFlowService::discover(&config).await {
                    Ok(flow) => {
                        tracing::info!(
                            issuer = %config.issuer_url,
                            "OIDC provider discovered"
                        );
                        Some(OidcState {
                            flow: Arc::new(flow),
                            default_roles: oidc_default_roles_from_env(),
                            success_redirect: std::env::var("AMBIENTOR_OIDC_SUCCESS_URL").ok(),
                            config,
                        })
                    }
                    Err(e) => {
                        tracing::warn!("OIDC discovery failed: {e}");
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        Ok(Self {
            db,
            auth,
            oidc,
            jwt,
            sse,
        })
    }

    pub fn audit_store(&self) -> Option<Arc<dyn AuditStore>> {
        self.db.as_ref().map(|d| d.audit.clone())
    }

    pub fn scan_store(&self) -> Option<Arc<dyn ScanStore>> {
        self.db.as_ref().map(|d| d.scan.clone())
    }

    pub fn dashboard_store(&self) -> Option<Arc<dyn DashboardStore>> {
        self.db.as_ref().map(|d| d.dashboard.clone())
    }

    pub fn applications_store(&self) -> Option<Arc<dyn ApplicationAssessmentStore>> {
        self.db.as_ref().map(|d| d.applications.clone())
    }

    pub fn verify_jwt(&self, token: &str) -> Result<Claims, ambientor_auth::jwt::JwtError> {
        self.jwt.verify(token)
    }
}
