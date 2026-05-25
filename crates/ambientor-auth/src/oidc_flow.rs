use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use openidconnect::TokenResponse;
use openidconnect::core::{
    CoreAuthenticationFlow, CoreClient, CoreIdTokenClaims, CoreProviderMetadata,
};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointMaybeSet, EndpointNotSet,
    EndpointSet, IssuerUrl, Nonce, PkceCodeChallenge, RedirectUrl, Scope,
};
use thiserror::Error;
use url::Url;

use crate::oidc::OidcConfig;

const PENDING_TTL: Duration = Duration::from_secs(600);

/// Client after OpenID Connect discovery (auth + token endpoints from metadata).
type DiscoveredOidcClient = CoreClient<
    EndpointSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointNotSet,
    EndpointMaybeSet,
    EndpointMaybeSet,
>;

#[derive(Debug, Error)]
pub enum OidcError {
    #[error("OIDC not configured: {0}")]
    NotConfigured(&'static str),
    #[error("discovery failed: {0}")]
    Discovery(String),
    #[error("invalid issuer URL: {0}")]
    InvalidIssuer(String),
    #[error("invalid redirect URL: {0}")]
    InvalidRedirect(String),
    #[error("token exchange failed: {0}")]
    TokenExchange(String),
    #[error("invalid or expired login state")]
    InvalidState,
    #[error("missing id token")]
    MissingIdToken,
}

#[derive(Clone, Debug)]
pub struct OidcIdentity {
    pub subject: String,
    pub username: String,
    pub email: Option<String>,
}

struct PendingLogin {
    pkce_verifier: openidconnect::PkceCodeVerifier,
    nonce: Nonce,
    created: Instant,
}

/// In-memory CSRF/PKCE store for the authorization code flow (single API replica).
pub struct OidcFlowService {
    client: DiscoveredOidcClient,
    http: openidconnect::reqwest::Client,
    pending: Mutex<HashMap<String, PendingLogin>>,
}

impl OidcFlowService {
    pub async fn discover(config: &OidcConfig) -> Result<Self, OidcError> {
        let issuer = IssuerUrl::new(config.issuer_url.clone())
            .map_err(|e| OidcError::InvalidIssuer(e.to_string()))?;
        let http = build_http_client();
        let metadata = CoreProviderMetadata::discover_async(issuer, &http)
            .await
            .map_err(|e| OidcError::Discovery(e.to_string()))?;

        let client_secret = std::env::var(&config.client_secret_env)
            .map_err(|_| OidcError::NotConfigured("client secret env var not set for OIDC"))?;

        let redirect = RedirectUrl::new(config.redirect_uri.clone())
            .map_err(|e| OidcError::InvalidRedirect(e.to_string()))?;

        let client = CoreClient::from_provider_metadata(
            metadata,
            ClientId::new(config.client_id.clone()),
            Some(ClientSecret::new(client_secret)),
        )
        .set_redirect_uri(redirect);

        Ok(Self {
            client,
            http,
            pending: Mutex::new(HashMap::new()),
        })
    }

    /// Start login: returns the IdP authorization URL to redirect the browser to.
    pub fn authorize_url(&self, config: &OidcConfig) -> Result<Url, OidcError> {
        self.gc_expired();
        let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
        let mut req = self.client.authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        );
        for scope in &config.scopes {
            req = req.add_scope(Scope::new(scope.clone()));
        }
        let (auth_url, csrf_token, nonce) = req.set_pkce_challenge(pkce_challenge).url();

        self.pending.lock().expect("oidc pending lock").insert(
            csrf_token.secret().to_string(),
            PendingLogin {
                pkce_verifier,
                nonce,
                created: Instant::now(),
            },
        );

        Ok(auth_url)
    }

    /// Complete login after IdP redirects back with `code` and `state`.
    pub async fn exchange_callback(
        &self,
        code: &str,
        state: &str,
    ) -> Result<OidcIdentity, OidcError> {
        self.gc_expired();
        let pending = self
            .pending
            .lock()
            .expect("oidc pending lock")
            .remove(state)
            .ok_or(OidcError::InvalidState)?;

        let token_response = self
            .client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .map_err(|e| OidcError::TokenExchange(e.to_string()))?
            .set_pkce_verifier(pending.pkce_verifier)
            .request_async(&self.http)
            .await
            .map_err(|e| OidcError::TokenExchange(e.to_string()))?;

        let id_token = token_response.id_token().ok_or(OidcError::MissingIdToken)?;
        let claims = id_token
            .claims(&self.client.id_token_verifier(), &pending.nonce)
            .map_err(|e| OidcError::TokenExchange(e.to_string()))?;

        let subject = claims.subject().to_string();
        let username = username_from_claims(claims);
        let email = claims.email().map(|c| c.to_string());

        Ok(OidcIdentity {
            subject,
            username,
            email,
        })
    }

    fn gc_expired(&self) {
        let mut guard = self.pending.lock().expect("oidc pending lock");
        guard.retain(|_, v| v.created.elapsed() < PENDING_TTL);
    }
}

fn username_from_claims(claims: &CoreIdTokenClaims) -> String {
    claims
        .preferred_username()
        .map(|c| c.as_str().to_string())
        .or_else(|| claims.email().map(|c| c.as_str().to_string()))
        .unwrap_or_else(|| format!("oidc-{}", claims.subject().as_str()))
}

fn build_http_client() -> openidconnect::reqwest::Client {
    openidconnect::reqwest::ClientBuilder::new()
        .redirect(openidconnect::reqwest::redirect::Policy::none())
        .build()
        .expect("reqwest client for OIDC")
}
