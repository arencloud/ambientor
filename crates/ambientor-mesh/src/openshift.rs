use ambientor_core::rules::RuleContext;
use ambientor_types::MeshFlavor;
use async_trait::async_trait;
use kube::Client;

use crate::backend::{MeshBackend, PreflightCheck};
use crate::inventory;
use crate::version::detect_istio_version;

pub struct OssmBackend;

#[async_trait]
impl MeshBackend for OssmBackend {
    fn flavor(&self) -> MeshFlavor {
        MeshFlavor::OSSM3
    }

    async fn detect_version(&self, client: &Client) -> anyhow::Result<Option<String>> {
        Ok(detect_istio_version(client).await)
    }

    async fn build_rule_context(&self, client: &Client) -> anyhow::Result<RuleContext> {
        inventory::collect_inventory(client, MeshFlavor::OSSM3, None).await
    }

    async fn preflight_checks(&self, client: &Client) -> anyhow::Result<Vec<PreflightCheck>> {
        let mut checks = inventory::common_preflight(client).await?;
        checks.extend(ossm_preflight(client).await?);
        Ok(checks)
    }
}

async fn ossm_preflight(client: &Client) -> anyhow::Result<Vec<PreflightCheck>> {
    let platform = ambientor_k8s::detect_platform(client).await?;
    let opts = crate::openshift_wizard::OpenShiftWizardOptions {
        ambientor_namespace: std::env::var("POD_NAMESPACE")
            .unwrap_or_else(|_| "ambientor-system".into()),
        operator_service_account: std::env::var("AMBIENTOR_OPERATOR_SA")
            .unwrap_or_else(|_| "ambientor-operator".into()),
        enroll_namespaces: crate::openshift_wizard::namespaces_needing_enrollment(client)
            .await
            .unwrap_or_default(),
    };
    let report = crate::openshift_wizard::run_wizard(client, &platform, &opts).await?;
    Ok(report
        .steps
        .into_iter()
        .map(|s| PreflightCheck {
            id: s.id,
            passed: s.passed,
            message: s.message,
            remediation: s.remediation,
        })
        .collect())
}
