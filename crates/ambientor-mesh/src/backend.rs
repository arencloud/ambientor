use ambientor_core::rules::RuleContext;
use ambientor_types::MeshFlavor;
use async_trait::async_trait;
use kube::Client;

use crate::inventory;
use crate::openshift;

#[async_trait]
pub trait MeshBackend: Send + Sync {
    fn flavor(&self) -> MeshFlavor;
    async fn detect_version(&self, client: &Client) -> anyhow::Result<Option<String>>;
    async fn build_rule_context(&self, client: &Client) -> anyhow::Result<RuleContext>;
    async fn preflight_checks(&self, client: &Client) -> anyhow::Result<Vec<PreflightCheck>>;
}

#[derive(Clone, Debug)]
pub struct PreflightCheck {
    pub id: String,
    pub passed: bool,
    pub message: String,
    pub remediation: Option<String>,
}

pub fn backend_for_flavor(flavor: MeshFlavor) -> Box<dyn MeshBackend> {
    match flavor {
        MeshFlavor::OSSM3 => Box::new(openshift::OssmBackend),
        MeshFlavor::UpstreamIstio => Box::new(UpstreamIstioBackend),
        MeshFlavor::GenericKubernetes | MeshFlavor::Unknown => Box::new(GenericBackend),
    }
}

pub struct UpstreamIstioBackend;

#[async_trait]
impl MeshBackend for UpstreamIstioBackend {
    fn flavor(&self) -> MeshFlavor {
        MeshFlavor::UpstreamIstio
    }

    async fn detect_version(&self, _client: &Client) -> anyhow::Result<Option<String>> {
        Ok(None)
    }

    async fn build_rule_context(&self, client: &Client) -> anyhow::Result<RuleContext> {
        inventory::collect_inventory(client, MeshFlavor::UpstreamIstio, None).await
    }

    async fn preflight_checks(&self, client: &Client) -> anyhow::Result<Vec<PreflightCheck>> {
        inventory::common_preflight(client).await
    }
}

pub struct GenericBackend;

#[async_trait]
impl MeshBackend for GenericBackend {
    fn flavor(&self) -> MeshFlavor {
        MeshFlavor::GenericKubernetes
    }

    async fn detect_version(&self, _client: &Client) -> anyhow::Result<Option<String>> {
        Ok(None)
    }

    async fn build_rule_context(&self, client: &Client) -> anyhow::Result<RuleContext> {
        inventory::collect_inventory(client, MeshFlavor::GenericKubernetes, None).await
    }

    async fn preflight_checks(&self, client: &Client) -> anyhow::Result<Vec<PreflightCheck>> {
        inventory::common_preflight(client).await
    }
}
