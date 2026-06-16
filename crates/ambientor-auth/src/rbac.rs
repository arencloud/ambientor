use casbin::{CoreApi, DefaultModel, Enforcer, MgmtApi};
use sqlx::PgPool;
use sqlx_adapter::SqlxAdapter;
use thiserror::Error;

/// RBAC with namespace domain: `enforce(subject, namespace, object, action)`.
const MODEL: &str = r#"
[request_definition]
r = sub, dom, obj, act

[policy_definition]
p = sub, dom, obj, act, eft

[role_definition]
g = _, _, _

[policy_effect]
e = some(where (p.eft == allow)) && !some(where (p.eft == deny))

[matchers]
m = g(r.sub, p.sub, r.dom) && keyMatch(r.dom, p.dom) && keyMatch(r.obj, p.obj) && regexMatch(r.act, p.act)
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
    pub async fn with_postgres(pool: PgPool) -> Result<Self, RbacError> {
        let adapter = SqlxAdapter::new_with_pool(pool.clone())
            .await
            .map_err(|e| RbacError::Casbin(e.to_string()))?;
        let model = DefaultModel::from_str(MODEL)
            .await
            .map_err(|e| RbacError::Casbin(e.to_string()))?;
        let mut enforcer = Enforcer::new(model, adapter)
            .await
            .map_err(|e| RbacError::Casbin(e.to_string()))?;

        if casbin_rule_count(&pool).await? == 0 {
            seed_defaults(&mut enforcer).await?;
            enforcer
                .save_policy()
                .await
                .map_err(|e| RbacError::Casbin(e.to_string()))?;
        }

        Ok(Self { enforcer })
    }

    /// In-memory enforcer for tests and when `DATABASE_URL` is unset.
    pub async fn with_defaults() -> Result<Self, RbacError> {
        use casbin::MemoryAdapter;
        let model = DefaultModel::from_str(MODEL)
            .await
            .map_err(|e| RbacError::Casbin(e.to_string()))?;
        let adapter = MemoryAdapter::default();
        let mut enforcer = Enforcer::new(model, adapter)
            .await
            .map_err(|e| RbacError::Casbin(e.to_string()))?;
        seed_defaults(&mut enforcer).await?;
        Ok(Self { enforcer })
    }

    pub async fn assign_role(
        &mut self,
        user: &str,
        role: &str,
        namespace: &str,
    ) -> Result<(), RbacError> {
        self.enforcer
            .add_grouping_policy(vec![
                user.to_string(),
                role.to_string(),
                namespace.to_string(),
            ])
            .await
            .map_err(|e| RbacError::Casbin(e.to_string()))?;
        self.enforcer
            .save_policy()
            .await
            .map_err(|e| RbacError::Casbin(e.to_string()))?;
        Ok(())
    }

    pub fn enforce(
        &self,
        sub: &str,
        namespace: &str,
        obj: &str,
        act: &str,
    ) -> Result<bool, RbacError> {
        self.enforcer
            .enforce((sub, namespace, obj, act))
            .map_err(|e| RbacError::Casbin(e.to_string()))
    }
}

async fn casbin_rule_count(pool: &PgPool) -> Result<i64, RbacError> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*)::bigint FROM casbin_rule")
        .fetch_one(pool)
        .await
        .map_err(|e| RbacError::Casbin(e.to_string()))?;
    Ok(count)
}

async fn seed_defaults(enforcer: &mut Enforcer) -> Result<(), RbacError> {
    let policies = [
        ("platform-admin", "*", "*", ".*", "allow"),
        ("mesh-admin", "*", "cluster/*", ".*", "allow"),
        (
            "migration-operator",
            "*",
            "rollout/*",
            "approve|execute|read",
            "allow",
        ),
        ("migration-operator", "*", "plan/*", "read|export", "allow"),
        // Alias for docs/CLI that say "operator"
        ("operator", "*", "rollout/*", "approve|execute|read", "allow"),
        ("operator", "*", "plan/*", "read|export", "allow"),
        ("viewer", "*", "*", "read", "allow"),
        ("auditor", "*", "audit/*", "read", "allow"),
    ];
    for (role, dom, obj, act, eft) in policies {
        enforcer
            .add_policy(vec![
                role.to_string(),
                dom.to_string(),
                obj.to_string(),
                act.to_string(),
                eft.to_string(),
            ])
            .await
            .map_err(|e| RbacError::Casbin(e.to_string()))?;
    }
    Ok(())
}

/// Build an object path for Casbin (`keyMatch` patterns).
/// Namespace is enforced via the Casbin domain argument, not duplicated in the path.
pub fn object_in_namespace(_namespace: &str, kind: &str, name: &str) -> String {
    format!("{kind}/{name}")
}

pub fn object_pattern(kind: &str, pattern: &str) -> String {
    format!("{kind}/{pattern}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn namespace_domain_enforces_rollout_approve() {
        let rbac = RbacEnforcer::with_defaults().await.unwrap();
        let object = object_in_namespace("bookinfo", "rollout", "bookinfo-reviews");
        assert!(
            rbac.enforce("migration-operator", "bookinfo", &object, "approve")
                .unwrap()
        );
        assert!(
            !rbac
                .enforce("viewer", "bookinfo", "rollout/bookinfo-reviews", "approve")
                .unwrap()
        );
        assert!(
            rbac.enforce("viewer", "bookinfo", "plan/bookinfo", "read")
                .unwrap()
        );
    }

    #[tokio::test]
    async fn grouping_policy_scopes_role_to_namespace() {
        let mut rbac = RbacEnforcer::with_defaults().await.unwrap();
        rbac.assign_role("alice", "migration-operator", "bookinfo")
            .await
            .unwrap();
        assert!(
            rbac.enforce("alice", "bookinfo", "rollout/x", "approve")
                .unwrap()
        );
        assert!(
            !rbac
                .enforce("alice", "default", "rollout/x", "approve")
                .unwrap()
        );
    }
}
