use ambientor_core::rules::RuleContext;
use ambientor_types::MeshFlavor;
use async_trait::async_trait;
use kube::Client;

use crate::backend::{MeshBackend, PreflightCheck};
use crate::inventory;

pub struct OssmBackend;

#[async_trait]
impl MeshBackend for OssmBackend {
    fn flavor(&self) -> MeshFlavor {
        MeshFlavor::OSSM3
    }

    async fn detect_version(&self, _client: &Client) -> anyhow::Result<Option<String>> {
        Ok(Some("ossm-3".into()))
    }

    async fn build_rule_context(&self, client: &Client) -> anyhow::Result<RuleContext> {
        let mut ctx = inventory::collect_inventory(client, MeshFlavor::OSSM3).await?;
        ctx.mesh_version = Some("ossm-3".into());
        Ok(ctx)
    }

    async fn preflight_checks(&self, client: &Client) -> anyhow::Result<Vec<PreflightCheck>> {
        let mut checks = inventory::common_preflight(client).await?;
        checks.extend(ossm_preflight(client).await?);
        Ok(checks)
    }
}

async fn ossm_preflight(client: &Client) -> anyhow::Result<Vec<PreflightCheck>> {
    let members = crate::platform_scan::collect_ossm_member_namespaces(client).await;
    let member_ok = !members.is_empty();
    Ok(vec![
        PreflightCheck {
            id: "ossm-member-roll".into(),
            passed: member_ok,
            message: if member_ok {
                format!(
                    "Found {} ServiceMeshMemberRoll member reference(s)",
                    members.len()
                )
            } else {
                "No ServiceMeshMemberRoll resources found".into()
            },
            remediation: Some(
                "Create ServiceMeshMemberRoll to enroll namespaces before ambient migration".into(),
            ),
        },
        PreflightCheck {
            id: "openshift-scc".into(),
            passed: true,
            message: "Verify ambientor ServiceAccount has required SCC (anyuid or custom)".into(),
            remediation: Some(
                "Grant restricted-v2 or custom SCC to ambientor-operator ServiceAccount".into(),
            ),
        },
    ])
}
