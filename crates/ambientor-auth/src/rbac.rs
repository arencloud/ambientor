use casbin::{CoreApi, DefaultModel, Enforcer, MemoryAdapter, MgmtApi};
use thiserror::Error;

const MODEL: &str = r#"
[request_definition]
r = sub, obj, act

[policy_definition]
p = sub, obj, act, eft

[role_definition]
g = _, _

[policy_effect]
e = some(where (p.eft == allow)) && !some(where (p.eft == deny))

[matchers]
m = g(r.sub, p.sub) && keyMatch(r.obj, p.obj) && regexMatch(r.act, p.act)
"#;

#[derive(Debug, Error)]
pub enum RbacError {
    #[error("casbin error: {0}")]
    Casbin(String),
}

pub struct RbacEnforcer {
    enforcer: Enforcer,
}

impl RbacEnforcer {
    pub async fn new() -> Result<Self, RbacError> {
        let model = DefaultModel::from_str(MODEL)
            .await
            .map_err(|e| RbacError::Casbin(e.to_string()))?;
        let adapter = MemoryAdapter::default();
        let enforcer = Enforcer::new(model, adapter)
            .await
            .map_err(|e| RbacError::Casbin(e.to_string()))?;
        Ok(Self { enforcer })
    }

    pub async fn with_defaults() -> Result<Self, RbacError> {
        let mut rbac = Self::new().await?;
        rbac.seed_defaults().await?;
        Ok(rbac)
    }

    async fn seed_defaults(&mut self) -> Result<(), RbacError> {
        let policies = [
            ("platform-admin", "*", ".*"),
            ("mesh-admin", "cluster/*", ".*"),
            ("migration-operator", "rollout/*", "approve|execute|read"),
            ("viewer", "*", "read"),
            ("auditor", "audit/*", "read"),
        ];
        for (role, obj, act) in policies {
            self.enforcer
                .add_policy(vec![
                    role.to_string(),
                    obj.to_string(),
                    act.to_string(),
                    "allow".to_string(),
                ])
                .await
                .map_err(|e| RbacError::Casbin(e.to_string()))?;
        }
        Ok(())
    }

    pub async fn assign_role(&mut self, user: &str, role: &str) -> Result<(), RbacError> {
        self.enforcer
            .add_grouping_policy(vec![user.to_string(), role.to_string()])
            .await
            .map_err(|e| RbacError::Casbin(e.to_string()))?;
        Ok(())
    }

    pub fn enforce(&mut self, sub: &str, obj: &str, act: &str) -> Result<bool, RbacError> {
        self.enforcer
            .enforce((sub, obj, act))
            .map_err(|e| RbacError::Casbin(e.to_string()))
    }
}
