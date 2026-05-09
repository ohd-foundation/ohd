//! OAuth 2.0 Device Authorization Grant client (RFC 8628).
//!
//! Per `spec/docs/design/auth.md` "CLI clients (`ohd-connect`,
//! `ohd-care`)", the device flow is the right shape for a terminal CLI:
//! the user is shown a URL + short code, confirms in their browser, and
//! the CLI polls until tokens come back.
//!
//! This module is shared by `ohd-connect` and `ohd-emergency` — neither
//! has anything CLI-specific in its OIDC path so we factored the whole
//! thing out verbatim. Discovery is hand-rolled because storage's AS
//! metadata at `/.well-known/oauth-authorization-server` may serve a
//! sparse document while v0.x stabilises; we fall back to
//! `/openid-configuration` on 404.

use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use oauth2::basic::BasicClient;
// `oauth2`'s reqwest helper is `reqwest::Client` re-exported with
// blocking redirects disabled. We use the same underlying type so we
// don't end up with two reqwest version trees in our deps.
use oauth2::reqwest::Client as Oauth2HttpClient;
use oauth2::{
    AuthUrl, ClientId, DeviceAuthorizationUrl, RefreshToken, Scope,
    StandardDeviceAuthorizationResponse, TokenUrl,
};
use serde::Deserialize;

/// Subset of OAuth 2.0 Authorization Server Metadata (RFC 8414) we need.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)] // optional fields are kept for future surfaces
pub struct DiscoveryDoc {
    pub issuer: String,
    pub token_endpoint: String,
    #[serde(default)]
    pub authorization_endpoint: Option<String>,
    pub device_authorization_endpoint: String,
    #[serde(default)]
    pub registration_endpoint: Option<String>,
}

/// Result of a successful device-flow exchange.
#[derive(Debug, Clone)]
#[allow(dead_code)] // scope is exposed for diagnostics; not consumed today
pub struct LoginTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_in_secs: Option<u64>,
    pub scope: Option<String>,
    pub oidc_subject: Option<String>,
    pub oidc_issuer: Option<String>,
}

/// One-stop builder for the device flow.
pub struct DeviceFlowClient {
    pub discovery: DiscoveryDoc,
    pub client_id: String,
    pub scope: String,
    http: Oauth2HttpClient,
}

impl DeviceFlowClient {
    pub async fn new(
        issuer: &str,
        client_id: &str,
        _client_secret: Option<&str>,
        scope: &str,
    ) -> Result<Self> {
        let discovery = discover(issuer).await?;
        let http = Oauth2HttpClient::builder()
            .build()
            .context("build oauth2 http client")?;
        Ok(Self {
            discovery,
            client_id: client_id.to_string(),
            scope: scope.to_string(),
            http,
        })
    }

    /// Run the full device flow: request a code, invoke `on_user_prompt`
    /// with the verification URL + code so the caller can render them,
    /// then poll the token endpoint until success or expiry.
    pub async fn run<F>(&self, on_user_prompt: F) -> Result<LoginTokens>
    where
        F: FnOnce(&StandardDeviceAuthorizationResponse),
    {
        let auth_url = self
            .discovery
            .authorization_endpoint
            .clone()
            .unwrap_or_else(|| self.discovery.token_endpoint.clone());
        let oauth_client = BasicClient::new(ClientId::new(self.client_id.clone()))
            .set_auth_uri(AuthUrl::new(auth_url).map_err(|e| anyhow!("invalid auth url: {e}"))?)
            .set_token_uri(
                TokenUrl::new(self.discovery.token_endpoint.clone())
                    .map_err(|e| anyhow!("invalid token url: {e}"))?,
            )
            .set_device_authorization_url(
                DeviceAuthorizationUrl::new(self.discovery.device_authorization_endpoint.clone())
                    .map_err(|e| anyhow!("invalid device-auth url: {e}"))?,
            );

        let device_resp: StandardDeviceAuthorizationResponse = oauth_client
            .exchange_device_code()
            .add_scope(Scope::new(self.scope.clone()))
            .request_async(&self.http)
            .await
            .map_err(|e| anyhow!("device-code request failed: {e}"))?;

        on_user_prompt(&device_resp);

        let token_result = oauth_client
            .exchange_device_access_token(&device_resp)
            .request_async(&self.http, tokio::time::sleep, None)
            .await
            .map_err(|e| anyhow!("device-flow poll failed: {e}"))?;

        use oauth2::TokenResponse as _;
        let access_token = token_result.access_token().secret().to_string();
        let refresh_token = token_result
            .refresh_token()
            .map(|t| t.secret().to_string());
        let expires_in_secs = token_result.expires_in().map(|d| d.as_secs());
        let scope = token_result
            .scopes()
            .map(|s| s.iter().map(|x| x.as_str()).collect::<Vec<_>>().join(" "));

        Ok(LoginTokens {
            access_token,
            refresh_token,
            expires_in_secs,
            scope,
            oidc_subject: None,
            oidc_issuer: Some(self.discovery.issuer.clone()),
        })
    }

    /// Refresh an access token (RFC 6749 §6).
    #[allow(dead_code)] // wired into a future `--silent-refresh` flag
    pub async fn refresh(&self, refresh_token: &str) -> Result<LoginTokens> {
        let auth_url = self
            .discovery
            .authorization_endpoint
            .clone()
            .unwrap_or_else(|| self.discovery.token_endpoint.clone());
        let oauth_client = BasicClient::new(ClientId::new(self.client_id.clone()))
            .set_auth_uri(AuthUrl::new(auth_url).map_err(|e| anyhow!("invalid auth url: {e}"))?)
            .set_token_uri(
                TokenUrl::new(self.discovery.token_endpoint.clone())
                    .map_err(|e| anyhow!("invalid token url: {e}"))?,
            );

        let token_result = oauth_client
            .exchange_refresh_token(&RefreshToken::new(refresh_token.to_string()))
            .request_async(&self.http)
            .await
            .map_err(|e| anyhow!("refresh failed: {e}"))?;
        use oauth2::TokenResponse as _;
        let access_token = token_result.access_token().secret().to_string();
        let new_refresh = token_result
            .refresh_token()
            .map(|t| t.secret().to_string());
        let expires_in_secs = token_result.expires_in().map(|d| d.as_secs());
        Ok(LoginTokens {
            access_token,
            refresh_token: new_refresh.or_else(|| Some(refresh_token.to_string())),
            expires_in_secs,
            scope: None,
            oidc_subject: None,
            oidc_issuer: Some(self.discovery.issuer.clone()),
        })
    }
}

// ---------------------------------------------------------------------------
// Discovery
// ---------------------------------------------------------------------------

/// Fetch `<issuer>/.well-known/oauth-authorization-server` (RFC 8414)
/// with fallback to `/openid-configuration` on 404.
pub async fn discover(issuer: &str) -> Result<DiscoveryDoc> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .context("build http client")?;

    let trimmed = issuer.trim_end_matches('/');
    let primary = format!("{trimmed}/.well-known/oauth-authorization-server");
    let fallback = format!("{trimmed}/.well-known/openid-configuration");

    let resp = client.get(&primary).send().await;
    let resp = match resp {
        Ok(r) if r.status().is_success() => r,
        Ok(r) if r.status().as_u16() == 404 => client
            .get(&fallback)
            .send()
            .await
            .with_context(|| format!("GET {fallback}"))?,
        Ok(r) => {
            return Err(anyhow!(
                "discovery: {} returned HTTP {}",
                primary,
                r.status()
            ))
        }
        Err(e) => return Err(anyhow!("discovery: {e}")),
    };
    if !resp.status().is_success() {
        return Err(anyhow!(
            "discovery: fallback {} returned HTTP {}",
            fallback,
            resp.status()
        ));
    }
    let doc: DiscoveryDoc = resp
        .json()
        .await
        .context("discovery: failed to parse JSON")?;
    Ok(doc)
}

// Re-export for callers that pattern-match on the type's full name.
pub use oauth2::StandardDeviceAuthorizationResponse as DeviceAuthorizationResponse;

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke-test that the discovery JSON parser handles a typical
    /// well-formed AS metadata document.
    #[test]
    fn parses_minimal_discovery_doc() {
        let json = r#"{
            "issuer": "https://issuer.example",
            "token_endpoint": "https://issuer.example/token",
            "authorization_endpoint": "https://issuer.example/authorize",
            "device_authorization_endpoint": "https://issuer.example/device"
        }"#;
        let doc: DiscoveryDoc = serde_json::from_str(json).unwrap();
        assert_eq!(doc.issuer, "https://issuer.example");
        assert_eq!(doc.token_endpoint, "https://issuer.example/token");
        assert_eq!(
            doc.device_authorization_endpoint,
            "https://issuer.example/device"
        );
        assert_eq!(
            doc.authorization_endpoint.as_deref(),
            Some("https://issuer.example/authorize")
        );
        assert!(doc.registration_endpoint.is_none());
    }

    /// AS metadata may omit `authorization_endpoint`; the DeviceFlowClient
    /// falls back to the token endpoint. Verify the field stays optional.
    #[test]
    fn parses_discovery_doc_without_authorization_endpoint() {
        let json = r#"{
            "issuer": "https://issuer.example",
            "token_endpoint": "https://issuer.example/token",
            "device_authorization_endpoint": "https://issuer.example/device"
        }"#;
        let doc: DiscoveryDoc = serde_json::from_str(json).unwrap();
        assert!(doc.authorization_endpoint.is_none());
    }
}
