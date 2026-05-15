//! OIDC relying-party: Authorization Code flow with PKCE against any
//! standards-compliant provider. Discovery-driven — the only per-provider
//! config is `(issuer, client_id, client_secret?, scopes)`.
//!
//! Used by the `/v1/auth/*` routes:
//!   start    → [`OidcClient::authorize_url`] (redirect the browser to the IdP)
//!   callback → [`OidcClient::exchange_code`] + [`OidcClient::verify_id_token`]

use crate::config::OidcProvider;
use crate::errors::ApiError;
use anyhow::{bail, Context};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use rand::Rng;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

const CACHE_TTL: Duration = Duration::from_secs(3600);
const PENDING_TTL: Duration = Duration::from_secs(600);

/// A login in flight: created at `/v1/auth/start`, consumed at the
/// callback. Keyed by the OAuth `state` value.
#[derive(Clone)]
pub struct PendingLogin {
    pub provider_id: String,
    pub code_verifier: String,
    created: Instant,
}

/// Shared map of in-flight logins.
pub type PendingLogins = Arc<Mutex<HashMap<String, PendingLogin>>>;

pub fn new_pending() -> PendingLogins {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Record a pending login under the OAuth `state` the IdP will echo back.
pub async fn insert_pending(
    map: &PendingLogins,
    state: &str,
    provider_id: &str,
    code_verifier: &str,
) {
    let mut guard = map.lock().await;
    guard.retain(|_, p| p.created.elapsed() < PENDING_TTL);
    guard.insert(
        state.to_string(),
        PendingLogin {
            provider_id: provider_id.to_string(),
            code_verifier: code_verifier.to_string(),
            created: Instant::now(),
        },
    );
}

/// Consume a pending login by `state` (single-use).
pub async fn take_pending(map: &PendingLogins, state: &str) -> Option<PendingLogin> {
    let mut guard = map.lock().await;
    let p = guard.remove(state)?;
    if p.created.elapsed() >= PENDING_TTL {
        return None;
    }
    Some(p)
}

/// Identity extracted from a verified `id_token`.
#[derive(Debug, Clone)]
pub struct VerifiedIdToken {
    pub iss: String,
    pub sub: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

impl VerifiedIdToken {
    /// Best display label: name, else email, else the subject.
    pub fn label(&self) -> String {
        self.name
            .clone()
            .or_else(|| self.email.clone())
            .unwrap_or_else(|| self.sub.clone())
    }
}

#[derive(Clone)]
pub struct OidcClient {
    http: reqwest::Client,
    cache: Arc<Mutex<HashMap<String, Cached>>>,
}

#[derive(Clone)]
struct Cached {
    discovery: Discovery,
    jwks: JwkSet,
    fetched_at: Instant,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
}

impl Default for OidcClient {
    fn default() -> Self {
        Self::new()
    }
}

impl OidcClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Build the IdP authorization URL for a login. Returns
    /// `(redirect_url, state, code_verifier)` — the caller stashes the
    /// verifier under `state`.
    pub async fn authorize_url(
        &self,
        provider: &OidcProvider,
        redirect_uri: &str,
    ) -> Result<(String, String, String), ApiError> {
        let disc = self.discovery(provider).await.map_err(upstream)?;
        let verifier = random_token(64);
        let challenge = pkce_challenge(&verifier);
        let state = random_token(32);
        let scope = if provider.scopes.is_empty() {
            "openid".to_string()
        } else {
            provider.scopes.join(" ")
        };
        let url = format!(
            "{auth}?response_type=code&client_id={cid}&redirect_uri={ruri}\
             &scope={scope}&state={state}&code_challenge={chal}&code_challenge_method=S256",
            auth = disc.authorization_endpoint,
            cid = urlencode(&provider.client_id),
            ruri = urlencode(redirect_uri),
            scope = urlencode(&scope),
            state = state,
            chal = challenge,
        );
        Ok((url, state, verifier))
    }

    /// Exchange an authorization `code` for an `id_token`.
    pub async fn exchange_code(
        &self,
        provider: &OidcProvider,
        code: &str,
        code_verifier: &str,
        redirect_uri: &str,
    ) -> Result<String, ApiError> {
        let disc = self.discovery(provider).await.map_err(upstream)?;
        let mut form = vec![
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", provider.client_id.as_str()),
            ("code_verifier", code_verifier),
        ];
        if !provider.client_secret.is_empty() {
            form.push(("client_secret", provider.client_secret.as_str()));
        }
        let resp = self
            .http
            .post(&disc.token_endpoint)
            .form(&form)
            .send()
            .await
            .map_err(|e| ApiError::Upstream(format!("token endpoint: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::Upstream(format!(
                "token endpoint returned {status}: {body}"
            )));
        }
        let tok: TokenResponse = resp
            .json()
            .await
            .map_err(|e| ApiError::Upstream(format!("token response parse: {e}")))?;
        tok.id_token
            .ok_or_else(|| ApiError::Upstream("token response had no id_token".into()))
    }

    /// Verify an `id_token`'s signature and standard claims.
    pub async fn verify_id_token(
        &self,
        provider: &OidcProvider,
        id_token: &str,
    ) -> Result<VerifiedIdToken, ApiError> {
        let header = decode_header(id_token)
            .map_err(|e| ApiError::Upstream(format!("id_token header: {e}")))?;
        if !supported_alg(header.alg) {
            return Err(ApiError::Upstream(format!(
                "unsupported id_token alg {:?}",
                header.alg
            )));
        }
        let jwks = self
            .jwks(provider, header.kid.as_deref())
            .await
            .map_err(upstream)?;
        let jwk = match &header.kid {
            Some(kid) => jwks
                .find(kid)
                .ok_or_else(|| ApiError::Upstream(format!("signing key {kid} not in JWKS")))?,
            None if jwks.keys.len() == 1 => &jwks.keys[0],
            None => {
                return Err(ApiError::Upstream(
                    "id_token has no kid and JWKS has multiple keys".into(),
                ))
            }
        };
        let key = DecodingKey::from_jwk(jwk)
            .map_err(|e| ApiError::Upstream(format!("decoding key: {e}")))?;
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[&provider.issuer]);
        validation.set_audience(&[&provider.client_id]);
        validation.leeway = 60;
        let data = decode::<IdClaims>(id_token, &key, &validation)
            .map_err(|e| ApiError::Upstream(format!("id_token verification failed: {e}")))?;
        let c = data.claims;
        Ok(VerifiedIdToken {
            iss: c.iss,
            sub: c.sub,
            email: c.email,
            name: c.name,
        })
    }

    async fn discovery(&self, provider: &OidcProvider) -> anyhow::Result<Discovery> {
        Ok(self.cached(provider, None).await?.discovery)
    }

    async fn jwks(&self, provider: &OidcProvider, kid_hint: Option<&str>) -> anyhow::Result<JwkSet> {
        Ok(self.cached(provider, kid_hint).await?.jwks)
    }

    /// Fetch (or reuse) the issuer's discovery doc + JWKS. A `kid` miss on
    /// a fresh cache entry forces one refresh — standard key-rotation
    /// handling.
    async fn cached(&self, provider: &OidcProvider, kid_hint: Option<&str>) -> anyhow::Result<Cached> {
        {
            let guard = self.cache.lock().await;
            if let Some(entry) = guard.get(&provider.issuer) {
                let fresh = entry.fetched_at.elapsed() < CACHE_TTL;
                let key_ok = kid_hint
                    .map(|k| entry.jwks.find(k).is_some())
                    .unwrap_or(true);
                if fresh && key_ok {
                    return Ok(entry.clone());
                }
            }
        }
        let entry = self.fetch(provider).await?;
        self.cache
            .lock()
            .await
            .insert(provider.issuer.clone(), entry.clone());
        Ok(entry)
    }

    async fn fetch(&self, provider: &OidcProvider) -> anyhow::Result<Cached> {
        let trimmed = provider.issuer.trim_end_matches('/');
        let disc_url = format!("{trimmed}/.well-known/openid-configuration");
        let discovery: Discovery = self
            .http
            .get(&disc_url)
            .send()
            .await
            .with_context(|| format!("GET {disc_url}"))?
            .error_for_status()
            .context("discovery status")?
            .json()
            .await
            .context("discovery parse")?;
        if discovery.issuer.trim_end_matches('/') != trimmed {
            bail!(
                "discovery issuer `{}` does not match configured `{}`",
                discovery.issuer,
                provider.issuer
            );
        }
        let jwks: JwkSet = self
            .http
            .get(&discovery.jwks_uri)
            .send()
            .await
            .with_context(|| format!("GET {}", discovery.jwks_uri))?
            .error_for_status()
            .context("jwks status")?
            .json()
            .await
            .context("jwks parse")?;
        Ok(Cached {
            discovery,
            jwks,
            fetched_at: Instant::now(),
        })
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    id_token: Option<String>,
}

#[derive(Deserialize)]
struct IdClaims {
    iss: String,
    sub: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

fn supported_alg(alg: Algorithm) -> bool {
    matches!(
        alg,
        Algorithm::RS256
            | Algorithm::RS384
            | Algorithm::RS512
            | Algorithm::PS256
            | Algorithm::PS384
            | Algorithm::PS512
            | Algorithm::ES256
            | Algorithm::ES384
            | Algorithm::EdDSA
    )
}

/// A URL-safe random token from the PKCE unreserved alphabet.
pub fn random_token(len: usize) -> String {
    const ALPHA: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::thread_rng();
    (0..len)
        .map(|_| ALPHA[rng.gen_range(0..ALPHA.len())] as char)
        .collect()
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn upstream(e: anyhow::Error) -> ApiError {
    ApiError::Upstream(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_challenge_is_stable_and_url_safe() {
        let v = "abc123";
        let c = pkce_challenge(v);
        assert_eq!(c, pkce_challenge(v));
        assert!(!c.contains('=') && !c.contains('+') && !c.contains('/'));
    }

    #[test]
    fn random_tokens_differ() {
        assert_ne!(random_token(32), random_token(32));
    }

    #[test]
    fn urlencode_escapes_reserved() {
        assert_eq!(urlencode("a b/c"), "a%20b%2Fc");
    }
}
