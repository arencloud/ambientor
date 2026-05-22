use anyhow::Context;
use kube::{
    Client,
    config::{KubeConfigOptions, Kubeconfig},
};

#[derive(Clone)]
pub struct K8sClient {
    pub client: Client,
}

impl K8sClient {
    pub async fn in_cluster() -> anyhow::Result<Self> {
        let config = kube::Config::infer()
            .await
            .context("failed to infer in-cluster kubeconfig")?;
        let client = Client::try_from(config).context("failed to build kube client")?;
        Ok(Self { client })
    }

    pub async fn from_kubeconfig(path: Option<&str>) -> anyhow::Result<Self> {
        let loader = match path {
            Some(p) => Kubeconfig::read_from(p).context("read kubeconfig")?,
            None => Kubeconfig::read().context("read default kubeconfig")?,
        };
        let config = kube::Config::from_custom_kubeconfig(loader, &KubeConfigOptions::default())
            .await
            .context("kubeconfig to config")?;
        let client = Client::try_from(config).context("failed to build kube client")?;
        Ok(Self { client })
    }
}
