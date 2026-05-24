use serde::{Deserialize, Serialize};

/// OIDC provider configuration (Keycloak, Okta, Azure AD presets).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OidcConfig {
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret_env: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub preset: Option<OidcPreset>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OidcPreset {
    Keycloak,
    Okta,
    AzureAd,
    Generic,
}

impl OidcConfig {
    pub fn keycloak(issuer_url: String, client_id: String, redirect_uri: String) -> Self {
        Self {
            issuer_url,
            client_id,
            client_secret_env: "AMBIENTOR_OIDC_CLIENT_SECRET".into(),
            redirect_uri,
            scopes: vec!["openid".into(), "profile".into(), "email".into()],
            preset: Some(OidcPreset::Keycloak),
        }
    }

    pub fn okta(issuer_url: String, client_id: String, redirect_uri: String) -> Self {
        Self {
            issuer_url,
            client_id,
            client_secret_env: "AMBIENTOR_OIDC_CLIENT_SECRET".into(),
            redirect_uri,
            scopes: vec!["openid".into(), "profile".into(), "email".into()],
            preset: Some(OidcPreset::Okta),
        }
    }

    pub fn azure_ad(tenant_id: &str, client_id: String, redirect_uri: String) -> Self {
        Self {
            issuer_url: format!("https://login.microsoftonline.com/{tenant_id}/v2.0"),
            client_id,
            client_secret_env: "AMBIENTOR_OIDC_CLIENT_SECRET".into(),
            redirect_uri,
            scopes: vec!["openid".into(), "profile".into(), "email".into()],
            preset: Some(OidcPreset::AzureAd),
        }
    }
}

/// Load OIDC settings when `AMBIENTOR_OIDC_ISSUER_URL`, `AMBIENTOR_OIDC_CLIENT_ID`, and
/// `AMBIENTOR_OIDC_REDIRECT_URI` are set. Client secret is read from the env var named by
/// `AMBIENTOR_OIDC_CLIENT_SECRET_ENV` (default `AMBIENTOR_OIDC_CLIENT_SECRET`).
pub fn oidc_config_from_env() -> Option<OidcConfig> {
    let issuer_url = std::env::var("AMBIENTOR_OIDC_ISSUER_URL").ok()?;
    let client_id = std::env::var("AMBIENTOR_OIDC_CLIENT_ID").ok()?;
    let redirect_uri = std::env::var("AMBIENTOR_OIDC_REDIRECT_URI").ok()?;
    let client_secret_env = std::env::var("AMBIENTOR_OIDC_CLIENT_SECRET_ENV")
        .unwrap_or_else(|_| "AMBIENTOR_OIDC_CLIENT_SECRET".into());
    let scopes = match std::env::var("AMBIENTOR_OIDC_SCOPES") {
        Ok(s) => s.split_whitespace().map(String::from).collect(),
        Err(_) => vec!["openid".into(), "profile".into(), "email".into()],
    };
    Some(OidcConfig {
        issuer_url,
        client_id,
        client_secret_env,
        redirect_uri,
        scopes,
        preset: None,
    })
}

/// Comma-separated roles for first-time OIDC users (`AMBIENTOR_OIDC_DEFAULT_ROLES`).
pub fn oidc_default_roles_from_env() -> Vec<String> {
    std::env::var("AMBIENTOR_OIDC_DEFAULT_ROLES")
        .map(|s| {
            s.split(',')
                .map(str::trim)
                .filter(|r| !r.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// Legacy manual authorize URL (no discovery/PKCE). Prefer [`crate::OidcFlowService`].
pub fn authorize_url(config: &OidcConfig, state: &str) -> String {
    format!(
        "{}/authorize?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}",
        config.issuer_url.trim_end_matches('/'),
        config.client_id,
        urlencoding::encode(&config.redirect_uri),
        urlencoding::encode(&config.scopes.join(" ")),
        state
    )
}
