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

/// Authorization URL builder placeholder until live OIDC discovery is wired.
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
