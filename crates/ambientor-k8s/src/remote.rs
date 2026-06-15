use std::collections::BTreeMap;

use ambientor_types::ClusterConnection;
use http::Uri;
use http::uri::InvalidUri;
use k8s_openapi::api::core::v1::Secret;
use kube::{
    Client,
    config::{AuthInfo, KubeConfigOptions, Kubeconfig},
};
use secrecy::SecretString;
use thiserror::Error;

use crate::K8sClient;

#[derive(Debug, Error)]
pub enum RemoteClientError {
    #[error("secret missing key: {0}")]
    MissingKey(&'static str),
    #[error("invalid secret data: {0}")]
    InvalidSecret(String),
    #[error("kubeconfig error: {0}")]
    Kubeconfig(String),
    #[error("client build error: {0}")]
    ClientBuild(String),
    #[error("kubernetes API error: {0}")]
    Api(#[from] kube::Error),
}

/// Stable cluster identifier for hub connections (used in scan persistence).
pub fn connection_cluster_ref(namespace: &str, name: &str) -> String {
    format!("{namespace}/{name}")
}

/// Parse `{namespace}/{name}` connection refs; returns `None` for hub-local refs like `in-cluster`.
pub fn parse_connection_cluster_ref(cluster_ref: &str) -> Option<(&str, &str)> {
    if cluster_ref == "in-cluster" || !cluster_ref.contains('/') {
        return None;
    }
    let (ns, name) = cluster_ref.split_once('/')?;
    if ns.is_empty() || name.is_empty() {
        None
    } else {
        Some((ns, name))
    }
}

/// Build a remote API client from a credentials Secret (`kubeconfig` or bearer token).
pub async fn client_from_secret(
    secret: &Secret,
    api_server_override: Option<&str>,
) -> Result<K8sClient, RemoteClientError> {
    let data = secret
        .data
        .as_ref()
        .ok_or(RemoteClientError::InvalidSecret(
            "secret has no data".into(),
        ))?;

    if let Some(kc) = data.get("kubeconfig") {
        return client_from_kubeconfig_bytes(kc.0.as_slice(), api_server_override).await;
    }

    client_from_token_secret(data, api_server_override).await
}

/// Load connection credentials from the hub cluster and return a client to the remote API.
pub async fn client_for_connection(
    hub: &Client,
    conn: &ClusterConnection,
) -> Result<K8sClient, RemoteClientError> {
    let ns = conn.metadata.namespace.as_deref().unwrap_or("default");
    let secret_ref = &conn.spec.credentials_secret_ref;
    let secret_ns = secret_ref.namespace.as_deref().unwrap_or(ns);
    let secret: Secret = kube::Api::namespaced(hub.clone(), secret_ns)
        .get(&secret_ref.name)
        .await?;
    client_from_secret(&secret, conn.spec.api_server.as_deref()).await
}

pub async fn verify_connectivity(client: &Client) -> Result<String, RemoteClientError> {
    let version = client.apiserver_version().await?;
    Ok(version.git_version)
}

async fn client_from_kubeconfig_bytes(
    kubeconfig: &[u8],
    api_server_override: Option<&str>,
) -> Result<K8sClient, RemoteClientError> {
    let text = std::str::from_utf8(kubeconfig)
        .map_err(|e| RemoteClientError::InvalidSecret(e.to_string()))?;
    let loader =
        Kubeconfig::from_yaml(text).map_err(|e| RemoteClientError::Kubeconfig(e.to_string()))?;
    let mut config = kube::Config::from_custom_kubeconfig(loader, &KubeConfigOptions::default())
        .await
        .map_err(|e| RemoteClientError::Kubeconfig(e.to_string()))?;
    if let Some(server) = api_server_override {
        let url: Uri = server
            .parse::<Uri>()
            .map_err(|e: InvalidUri| RemoteClientError::ClientBuild(e.to_string()))?;
        config.cluster_url = url;
    }
    let client =
        Client::try_from(config).map_err(|e| RemoteClientError::ClientBuild(e.to_string()))?;
    Ok(K8sClient { client })
}

async fn client_from_token_secret(
    data: &BTreeMap<String, k8s_openapi::ByteString>,
    api_server_override: Option<&str>,
) -> Result<K8sClient, RemoteClientError> {
    let token = data.get("token").ok_or(RemoteClientError::MissingKey(
        "token (or kubeconfig) required in credentials secret",
    ))?;
    let server = api_server_override
        .map(str::to_string)
        .or_else(|| {
            data.get("server").and_then(|b| {
                std::str::from_utf8(b.0.as_slice())
                    .ok()
                    .map(|s| s.trim().to_string())
            })
        })
        .ok_or(RemoteClientError::MissingKey(
            "spec.apiServer or secret key 'server' required for token auth",
        ))?;
    let token = std::str::from_utf8(token.0.as_slice())
        .map_err(|e| RemoteClientError::InvalidSecret(e.to_string()))?
        .trim()
        .to_string();

    let url: Uri = server
        .parse()
        .map_err(|e| RemoteClientError::ClientBuild(format!("invalid api server URL: {e}")))?;
    let mut config = kube::Config::new(url);
    config.auth_info = AuthInfo {
        token: Some(SecretString::new(token.into())),
        ..Default::default()
    };

    if let Some(ca) = data.get("ca.crt").or_else(|| data.get("ca-bundle")) {
        config.root_cert = Some(vec![ca.0.clone()]);
    }

    let client =
        Client::try_from(config).map_err(|e| RemoteClientError::ClientBuild(e.to_string()))?;
    Ok(K8sClient { client })
}

#[cfg(test)]
mod tests {
    use k8s_openapi::ByteString;

    use super::*;

    fn secret_with(data: BTreeMap<String, Vec<u8>>) -> Secret {
        Secret {
            data: Some(data.into_iter().map(|(k, v)| (k, ByteString(v))).collect()),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn rejects_empty_secret() {
        let secret = Secret::default();
        assert!(matches!(
            client_from_secret(&secret, None).await,
            Err(RemoteClientError::InvalidSecret(_))
        ));
    }

    #[tokio::test]
    async fn token_auth_requires_api_server() {
        let secret = secret_with(BTreeMap::from([("token".into(), b"tok".to_vec())]));
        assert!(matches!(
            client_from_secret(&secret, None).await,
            Err(RemoteClientError::MissingKey(_))
        ));
    }

    #[test]
    fn parse_connection_ref_roundtrip() {
        assert_eq!(
            parse_connection_cluster_ref("ambientor-system/cl02"),
            Some(("ambientor-system", "cl02"))
        );
        assert_eq!(parse_connection_cluster_ref("in-cluster"), None);
    }
}
