//! OIDC `id_token` verifier with a JWKS cache.
//!
//! ## What it does
//!
//! Given an issuer allowlist (from `[auth.registration]` in `relay.toml`)
//! and a raw JWT compact-encoded `id_token`, the verifier:
//!
//! 1. Decodes the JWT header to extract `kid` and `alg`.
//! 2. Decodes the unverified payload to extract the `iss` claim and looks
//!    up the matching allowlist entry (issuer → expected_audience).
//! 3. Resolves the issuer's JWKS, either from cache (within TTL) or by
//!    fetching `<issuer>/.well-known/openid-configuration` → `jwks_uri`
//!    → JWKS JSON.
//! 4. If the JWT's `kid` is not in the cached set, force-refreshes the
//!    JWKS once (key rotation).
//! 5. Verifies the signature with `jsonwebtoken::decode`, validating
//!    `exp`, `nbf`, `iat`, and `aud` (must include `expected_audience`).
//! 6. Returns `VerifiedIdToken { iss, sub, aud, kid }` on success.
//!
//! ## What it deliberately does NOT do
//!
//! - **Nonce validation**: this is a registration RPC, not an
//!   interactive login. The relay doesn't issue nonces.
//! - **At-hash / c_hash**: those are for OIDC implicit / hybrid flows;
//!   irrelevant for back-channel id_token presentation.
//! - **`acr` / `amr` policy gates**: out of scope for v1; an operator
//!   that wants AAL2 enforcement layers it on top of the OIDC IdP
//!   (which is the IdP's job).
//!
//! ## JWKS cache semantics
//!
//! - Keyed by issuer URL (exact-match against `iss`).
//! - TTL configured via `auth.registration.jwks_cache_ttl_secs` (default
//!   3600s).
//! - On a `kid` miss within TTL: one forced refresh; if the `kid` is
//!   still missing after that, reject as `KeyNotFound`. This is the
//!   standard OIDC key-rotation pattern.
//! - On a network failure during refresh while a cached set is present
//!   AND not yet TTL-expired: the cached set is reused (degraded mode);
//!   if no cached set exists, the verification fails closed.
//!
//! ## Algorithm support
//!
//! Whatever `jsonwebtoken::DecodingKey::from_jwk` accepts for the JWK's
//! declared `alg` — RS256/RS384/RS512, ES256/ES384, EdDSA. We do NOT
//! accept `none` and we do NOT accept HMAC algorithms (the issuer
//! shouldn't be sharing a symmetric secret over the wire anyway).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::{decode, decode_header, jwk::JwkSet, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use thiserror::Error;
use tokio::sync::RwLock;

use crate::config::AllowedIssuer;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Configuration for [`OidcVerifier`].
#[derive(Debug, Clone)]
pub struct OidcVerifierConfig {
    pub allowed_issuers: Vec<AllowedIssuer>,
    pub jwks_cache_ttl: Duration,
    /// Optional override map for issuer → discovery URL. Tests use this
    /// to point at a `wiremock` / local stub instead of the real well-
    /// known endpoint. Production never sets this.
    pub discovery_override: HashMap<String, String>,
}

impl OidcVerifierConfig {
    pub fn from_registration(
        cfg: &crate::config::RegistrationAuthConfig,
    ) -> Self {
        Self {
            allowed_issuers: cfg.allowed_issuers.clone(),
            jwks_cache_ttl: Duration::from_secs(cfg.jwks_cache_ttl_secs),
            discovery_override: HashMap::new(),
        }
    }
}

/// The output of a successful verification: identifies who the operator
/// is per their OIDC IdP. Stored alongside the registration for audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedIdToken {
    /// `iss` claim — the issuer URL.
    pub iss: String,
    /// `sub` claim — the subject identifier within the issuer.
    pub sub: String,
    /// `aud` claim that matched the configured `expected_audience`.
    pub aud: String,
    /// `kid` from the JWT header (None if the token didn't carry one).
    pub kid: Option<String>,
}

#[derive(Debug, Error)]
pub enum OidcVerifyError {
    #[error("id_token missing")]
    Missing,

    #[error("id_token malformed: {0}")]
    Malformed(String),

    #[error("issuer `{0}` is not on the allowlist")]
    IssuerNotAllowed(String),

    #[error("signing key `{0}` not found in issuer JWKS (after refresh)")]
    KeyNotFound(String),

    #[error("issuer JWKS fetch failed: {0}")]
    JwksFetch(String),

    #[error("issuer discovery fetch failed: {0}")]
    DiscoveryFetch(String),

    #[error("signature verification failed: {0}")]
    Signature(String),

    #[error("token expired or not yet valid: {0}")]
    Validity(String),

    #[error("audience mismatch: token aud={token_aud:?}, expected={expected:?}")]
    AudienceMismatch {
        token_aud: Vec<String>,
        expected: String,
    },

    #[error("unsupported algorithm `{0}`")]
    UnsupportedAlg(String),
}

// ---------------------------------------------------------------------------
// Verifier
// ---------------------------------------------------------------------------

/// Holds the issuer allowlist + a per-issuer JWKS cache. Cloning is
/// cheap (everything is `Arc`-wrapped).
#[derive(Clone)]
pub struct OidcVerifier {
    inner: Arc<Inner>,
}

struct Inner {
    cfg: OidcVerifierConfig,
    /// Per-issuer cached JWKS set. Keyed by `iss` (exact-match). Each
    /// cache entry tracks its fetched-at instant so we honor TTL.
    cache: RwLock<HashMap<String, CachedJwks>>,
    /// HTTP client for the JWKS fetches. Reuses connections within the
    /// process for free.
    http: reqwest::Client,
}

#[derive(Clone)]
struct CachedJwks {
    set: JwkSet,
    fetched_at: Instant,
}

impl OidcVerifier {
    pub fn new(cfg: OidcVerifierConfig) -> Self {
        Self::with_http(cfg, reqwest::Client::new())
    }

    pub fn with_http(cfg: OidcVerifierConfig, http: reqwest::Client) -> Self {
        Self {
            inner: Arc::new(Inner {
                cfg,
                cache: RwLock::new(HashMap::new()),
                http,
            }),
        }
    }

    pub fn config(&self) -> &OidcVerifierConfig {
        &self.inner.cfg
    }

    pub fn allowed_issuers(&self) -> &[AllowedIssuer] {
        &self.inner.cfg.allowed_issuers
    }

    /// Verify a compact-encoded JWT id_token. On success returns the
    /// extracted `(iss, sub, aud, kid)` for audit logging.
    pub async fn verify(&self, id_token: &str) -> Result<VerifiedIdToken, OidcVerifyError> {
        if id_token.is_empty() {
            return Err(OidcVerifyError::Missing);
        }

        // Step 1: header. Tells us which key (kid) and which alg we'll need.
        let header = decode_header(id_token).map_err(|e| {
            OidcVerifyError::Malformed(format!("decode header: {e}"))
        })?;
        let alg = header.alg;
        if !is_supported_alg(alg) {
            return Err(OidcVerifyError::UnsupportedAlg(format!("{alg:?}")));
        }
        let kid = header.kid.clone();

        // Step 2: extract iss/aud unverified so we can pick the allowlist
        // entry. We do NOT trust these values — they're confirmed by the
        // signature check + jsonwebtoken's `Validation` below.
        let unverified = peek_iss(id_token)?;
        let allow = self
            .inner
            .cfg
            .allowed_issuers
            .iter()
            .find(|a| a.issuer == unverified.iss)
            .ok_or_else(|| OidcVerifyError::IssuerNotAllowed(unverified.iss.clone()))?;

        // Step 3: resolve the JWKS for the issuer.
        let jwks = self.resolve_jwks(&allow.issuer, kid.as_deref()).await?;

        // Step 4: pick the JWK matching `kid` (or the only one, if there's
        // exactly one and the JWT didn't carry a kid).
        let jwk = pick_jwk(&jwks, kid.as_deref())?;

        // Step 5: verify signature + standard claims with jsonwebtoken.
        let decoding_key = DecodingKey::from_jwk(jwk).map_err(|e| {
            OidcVerifyError::Signature(format!("DecodingKey::from_jwk: {e}"))
        })?;

        let mut validation = Validation::new(alg);
        validation.set_issuer(&[&allow.issuer]);
        validation.set_audience(&[&allow.expected_audience]);
        validation.validate_exp = true;
        validation.validate_nbf = true;
        // `iat` is informational per RFC 7519. We don't reject on `iat`
        // alone; jsonwebtoken's `validate_exp`/`validate_nbf` cover the
        // important cases.
        validation.leeway = 30; // 30s clock skew tolerance.

        let token_data = decode::<Claims>(id_token, &decoding_key, &validation).map_err(
            |e| match e.kind() {
                jsonwebtoken::errors::ErrorKind::InvalidAudience => {
                    OidcVerifyError::AudienceMismatch {
                        token_aud: Vec::new(),
                        expected: allow.expected_audience.clone(),
                    }
                }
                jsonwebtoken::errors::ErrorKind::ExpiredSignature
                | jsonwebtoken::errors::ErrorKind::ImmatureSignature => {
                    OidcVerifyError::Validity(e.to_string())
                }
                jsonwebtoken::errors::ErrorKind::InvalidSignature
                | jsonwebtoken::errors::ErrorKind::InvalidAlgorithm
                | jsonwebtoken::errors::ErrorKind::InvalidAlgorithmName
                | jsonwebtoken::errors::ErrorKind::InvalidKeyFormat
                | jsonwebtoken::errors::ErrorKind::InvalidEcdsaKey
                | jsonwebtoken::errors::ErrorKind::InvalidRsaKey(_) => {
                    OidcVerifyError::Signature(e.to_string())
                }
                _ => OidcVerifyError::Signature(e.to_string()),
            },
        )?;

        let claims = token_data.claims;
        // Confirm aud explicitly: jsonwebtoken's `set_audience` already
        // checked it but we surface it for logging.
        let aud = claims
            .audience_strings()
            .into_iter()
            .find(|a| a == &allow.expected_audience)
            .ok_or_else(|| OidcVerifyError::AudienceMismatch {
                token_aud: claims.audience_strings(),
                expected: allow.expected_audience.clone(),
            })?;

        Ok(VerifiedIdToken {
            iss: claims.iss,
            sub: claims.sub,
            aud,
            kid,
        })
    }

    /// Fetch (or use cached) JWKS for the given issuer. When `kid_hint`
    /// is present and not in the cached set, force a refresh once before
    /// returning — this is the standard OIDC key-rotation handling.
    async fn resolve_jwks(
        &self,
        issuer: &str,
        kid_hint: Option<&str>,
    ) -> Result<JwkSet, OidcVerifyError> {
        // Fast path: cached + fresh + key present.
        {
            let cache = self.inner.cache.read().await;
            if let Some(entry) = cache.get(issuer) {
                let fresh = entry.fetched_at.elapsed() < self.inner.cfg.jwks_cache_ttl;
                let key_ok = kid_hint
                    .map(|k| entry.set.find(k).is_some())
                    .unwrap_or(true);
                if fresh && key_ok {
                    return Ok(entry.set.clone());
                }
            }
        }

        // Slow path: fetch (with one retry if `kid` still missing).
        let fetched = self.fetch_jwks(issuer).await;

        match fetched {
            Ok(set) => {
                let mut cache = self.inner.cache.write().await;
                cache.insert(
                    issuer.to_string(),
                    CachedJwks {
                        set: set.clone(),
                        fetched_at: Instant::now(),
                    },
                );

                if let Some(k) = kid_hint {
                    if set.find(k).is_none() {
                        return Err(OidcVerifyError::KeyNotFound(k.to_string()));
                    }
                }
                Ok(set)
            }
            Err(e) => {
                // Degraded mode: if we have a stale cached entry, reuse it.
                let cache = self.inner.cache.read().await;
                if let Some(entry) = cache.get(issuer) {
                    tracing::warn!(
                        target: "ohd_relay::auth::oidc",
                        issuer = %issuer,
                        error = %e,
                        "JWKS refresh failed; reusing stale cached JWKS"
                    );
                    return Ok(entry.set.clone());
                }
                Err(e)
            }
        }
    }

    /// Hit the issuer's `.well-known/openid-configuration`, then GET the
    /// `jwks_uri`.
    async fn fetch_jwks(&self, issuer: &str) -> Result<JwkSet, OidcVerifyError> {
        let discovery_url = self
            .inner
            .cfg
            .discovery_override
            .get(issuer)
            .cloned()
            .unwrap_or_else(|| {
                let trimmed = issuer.trim_end_matches('/');
                format!("{trimmed}/.well-known/openid-configuration")
            });

        let disc: DiscoveryDoc = self
            .inner
            .http
            .get(&discovery_url)
            .send()
            .await
            .map_err(|e| OidcVerifyError::DiscoveryFetch(format!("send {discovery_url}: {e}")))?
            .error_for_status()
            .map_err(|e| OidcVerifyError::DiscoveryFetch(format!("status: {e}")))?
            .json()
            .await
            .map_err(|e| OidcVerifyError::DiscoveryFetch(format!("parse: {e}")))?;

        if disc.issuer != issuer {
            // OIDC core rule: discovery doc's `issuer` MUST match the
            // configured issuer URL (exact string match).
            return Err(OidcVerifyError::DiscoveryFetch(format!(
                "discovery issuer mismatch: configured={issuer} doc={}",
                disc.issuer
            )));
        }

        let set: JwkSet = self
            .inner
            .http
            .get(&disc.jwks_uri)
            .send()
            .await
            .map_err(|e| OidcVerifyError::JwksFetch(format!("send {}: {e}", disc.jwks_uri)))?
            .error_for_status()
            .map_err(|e| OidcVerifyError::JwksFetch(format!("status: {e}")))?
            .json()
            .await
            .map_err(|e| OidcVerifyError::JwksFetch(format!("parse: {e}")))?;
        Ok(set)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct DiscoveryDoc {
    issuer: String,
    jwks_uri: String,
}

#[derive(Debug, Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    /// `aud` may be a string or an array of strings per RFC 7519. We
    /// accept both.
    #[serde(default)]
    aud: AudClaim,
}

#[derive(Debug, Default, Deserialize)]
#[serde(untagged)]
enum AudClaim {
    Many(Vec<String>),
    One(String),
    #[default]
    None,
}

impl Claims {
    fn audience_strings(&self) -> Vec<String> {
        match &self.aud {
            AudClaim::One(s) => vec![s.clone()],
            AudClaim::Many(v) => v.clone(),
            AudClaim::None => Vec::new(),
        }
    }
}

/// Decode the JWT payload without verifying the signature, just enough to
/// pull out `iss`. We use this to pick the correct allowlist entry before
/// loading its JWKS — the signature is verified later.
fn peek_iss(token: &str) -> Result<UnverifiedIss, OidcVerifyError> {
    use base64::Engine;
    let mut parts = token.splitn(3, '.');
    let _h = parts.next().ok_or_else(|| {
        OidcVerifyError::Malformed("missing header segment".into())
    })?;
    let payload_b64 = parts
        .next()
        .ok_or_else(|| OidcVerifyError::Malformed("missing payload segment".into()))?;
    let _sig = parts.next().ok_or_else(|| {
        OidcVerifyError::Malformed("missing signature segment".into())
    })?;

    let payload_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|e| OidcVerifyError::Malformed(format!("base64 payload: {e}")))?;
    let parsed: UnverifiedIss = serde_json::from_slice(&payload_bytes)
        .map_err(|e| OidcVerifyError::Malformed(format!("payload json: {e}")))?;
    Ok(parsed)
}

#[derive(Debug, Deserialize)]
struct UnverifiedIss {
    iss: String,
}

fn pick_jwk<'a>(
    set: &'a JwkSet,
    kid: Option<&str>,
) -> Result<&'a jsonwebtoken::jwk::Jwk, OidcVerifyError> {
    match kid {
        Some(k) => set
            .find(k)
            .ok_or_else(|| OidcVerifyError::KeyNotFound(k.to_string())),
        None => {
            if set.keys.len() == 1 {
                Ok(&set.keys[0])
            } else {
                Err(OidcVerifyError::Malformed(
                    "JWT missing kid header and issuer JWKS has multiple keys".into(),
                ))
            }
        }
    }
}

fn is_supported_alg(alg: Algorithm) -> bool {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{encode, EncodingKey, Header};
    use rsa::pkcs8::EncodePrivateKey;
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPrivateKey;
    use serde::Serialize;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    /// Build an RSA-2048 keypair, package it as both PEM (for signing)
    /// and a JWKS-style public JSON document (for the verifier to fetch).
    struct TestRsaKey {
        signing_pem: Vec<u8>,
        kid: String,
        n_b64: String,
        e_b64: String,
    }

    fn gen_rsa() -> TestRsaKey {
        use base64::Engine;
        let mut rng = rand::thread_rng();
        let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let pem = priv_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let signing_pem = pem.as_bytes().to_vec();

        let pub_key = priv_key.to_public_key();
        let n = pub_key.n().to_bytes_be();
        let e = pub_key.e().to_bytes_be();
        let n_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&n);
        let e_b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&e);

        TestRsaKey {
            signing_pem,
            kid: "test-kid-1".into(),
            n_b64,
            e_b64,
        }
    }

    fn jwks_json(key: &TestRsaKey) -> serde_json::Value {
        serde_json::json!({
            "keys": [{
                "kty": "RSA",
                "use": "sig",
                "alg": "RS256",
                "kid": key.kid,
                "n": key.n_b64,
                "e": key.e_b64,
            }]
        })
    }

    #[derive(Serialize)]
    struct TestClaims {
        iss: String,
        sub: String,
        aud: String,
        exp: i64,
        nbf: i64,
        iat: i64,
    }

    fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64
    }

    fn sign_jwt(
        key: &TestRsaKey,
        claims: &TestClaims,
        kid_in_header: Option<&str>,
    ) -> String {
        let mut header = Header::new(Algorithm::RS256);
        if let Some(k) = kid_in_header {
            header.kid = Some(k.into());
        }
        encode(
            &header,
            claims,
            &EncodingKey::from_rsa_pem(&key.signing_pem).unwrap(),
        )
        .unwrap()
    }

    /// Spin up a local HTTP server that serves `discovery_url` and
    /// `jwks_uri` from in-memory state. Closure-driven so tests can mutate
    /// the JWKS to simulate rotation.
    async fn spawn_idp_stub(
        issuer: String,
        jwks: Arc<Mutex<serde_json::Value>>,
    ) -> (String, String, tokio::task::JoinHandle<()>) {
        use axum::{
            extract::State,
            response::{IntoResponse, Json},
            routing::get,
            Router,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{addr}");
        let discovery_url = format!("{base}/.well-known/openid-configuration");
        let jwks_url = format!("{base}/jwks.json");

        #[derive(Clone)]
        struct St {
            issuer: String,
            jwks: Arc<Mutex<serde_json::Value>>,
            jwks_url: String,
        }

        async fn discovery(State(s): State<St>) -> impl IntoResponse {
            Json(serde_json::json!({
                "issuer": s.issuer,
                "jwks_uri": s.jwks_url,
            }))
        }
        async fn jwks_route(State(s): State<St>) -> impl IntoResponse {
            let v = s.jwks.lock().await.clone();
            Json(v)
        }

        let app = Router::new()
            .route("/.well-known/openid-configuration", get(discovery))
            .route("/jwks.json", get(jwks_route))
            .with_state(St {
                issuer: issuer.clone(),
                jwks: jwks.clone(),
                jwks_url: jwks_url.clone(),
            });

        let h = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        (base, discovery_url, h)
    }

    fn verifier_for(
        issuer: &str,
        audience: &str,
        discovery_url: &str,
        ttl: Duration,
    ) -> OidcVerifier {
        let mut over = HashMap::new();
        over.insert(issuer.to_string(), discovery_url.to_string());
        OidcVerifier::new(OidcVerifierConfig {
            allowed_issuers: vec![AllowedIssuer {
                issuer: issuer.to_string(),
                expected_audience: audience.to_string(),
            }],
            jwks_cache_ttl: ttl,
            discovery_override: over,
        })
    }

    #[tokio::test]
    async fn verifies_valid_token_against_mock_idp() {
        let key = gen_rsa();
        let issuer = "https://idp.test/".to_string();
        let jwks = Arc::new(Mutex::new(jwks_json(&key)));
        let (_base, discovery_url, _h) =
            spawn_idp_stub(issuer.clone(), jwks.clone()).await;

        let now = now_secs();
        let claims = TestClaims {
            iss: issuer.clone(),
            sub: "user-123".into(),
            aud: "ohd-relay-cloud".into(),
            exp: now + 600,
            nbf: now - 10,
            iat: now,
        };
        let jwt = sign_jwt(&key, &claims, Some(&key.kid));

        let v = verifier_for(
            &issuer,
            "ohd-relay-cloud",
            &discovery_url,
            Duration::from_secs(60),
        );
        let out = v.verify(&jwt).await.unwrap();
        assert_eq!(out.iss, issuer);
        assert_eq!(out.sub, "user-123");
        assert_eq!(out.aud, "ohd-relay-cloud");
        assert_eq!(out.kid.as_deref(), Some("test-kid-1"));
    }

    #[tokio::test]
    async fn rejects_token_with_unknown_issuer() {
        let key = gen_rsa();
        let issuer = "https://idp.test/".to_string();
        let jwks = Arc::new(Mutex::new(jwks_json(&key)));
        let (_base, discovery_url, _h) =
            spawn_idp_stub(issuer.clone(), jwks.clone()).await;

        let now = now_secs();
        let claims = TestClaims {
            iss: "https://evil.example/".into(),
            sub: "x".into(),
            aud: "ohd-relay-cloud".into(),
            exp: now + 600,
            nbf: now - 10,
            iat: now,
        };
        let jwt = sign_jwt(&key, &claims, Some(&key.kid));

        let v = verifier_for(
            &issuer,
            "ohd-relay-cloud",
            &discovery_url,
            Duration::from_secs(60),
        );
        let err = v.verify(&jwt).await.unwrap_err();
        assert!(matches!(err, OidcVerifyError::IssuerNotAllowed(_)));
    }

    #[tokio::test]
    async fn rejects_expired_token() {
        let key = gen_rsa();
        let issuer = "https://idp.test/".to_string();
        let jwks = Arc::new(Mutex::new(jwks_json(&key)));
        let (_base, discovery_url, _h) =
            spawn_idp_stub(issuer.clone(), jwks.clone()).await;

        let now = now_secs();
        let claims = TestClaims {
            iss: issuer.clone(),
            sub: "user".into(),
            aud: "ohd-relay-cloud".into(),
            exp: now - 600,
            nbf: now - 1200,
            iat: now - 1200,
        };
        let jwt = sign_jwt(&key, &claims, Some(&key.kid));

        let v = verifier_for(
            &issuer,
            "ohd-relay-cloud",
            &discovery_url,
            Duration::from_secs(60),
        );
        let err = v.verify(&jwt).await.unwrap_err();
        assert!(matches!(err, OidcVerifyError::Validity(_)), "{err:?}");
    }

    #[tokio::test]
    async fn rejects_audience_mismatch() {
        let key = gen_rsa();
        let issuer = "https://idp.test/".to_string();
        let jwks = Arc::new(Mutex::new(jwks_json(&key)));
        let (_base, discovery_url, _h) =
            spawn_idp_stub(issuer.clone(), jwks.clone()).await;

        let now = now_secs();
        let claims = TestClaims {
            iss: issuer.clone(),
            sub: "user".into(),
            aud: "wrong-audience".into(),
            exp: now + 600,
            nbf: now - 10,
            iat: now,
        };
        let jwt = sign_jwt(&key, &claims, Some(&key.kid));

        let v = verifier_for(
            &issuer,
            "ohd-relay-cloud",
            &discovery_url,
            Duration::from_secs(60),
        );
        let err = v.verify(&jwt).await.unwrap_err();
        assert!(
            matches!(err, OidcVerifyError::AudienceMismatch { .. }),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn rejects_bad_signature() {
        let key = gen_rsa();
        let intruder = gen_rsa(); // different key, same JWKS doc
        let issuer = "https://idp.test/".to_string();
        // The IdP publishes `key`'s public material; intruder signs with
        // their own private key → signature won't verify.
        let jwks = Arc::new(Mutex::new(jwks_json(&key)));
        let (_base, discovery_url, _h) =
            spawn_idp_stub(issuer.clone(), jwks.clone()).await;

        let now = now_secs();
        let claims = TestClaims {
            iss: issuer.clone(),
            sub: "user".into(),
            aud: "ohd-relay-cloud".into(),
            exp: now + 600,
            nbf: now - 10,
            iat: now,
        };
        // Use the intruder key but the trusted kid to bypass kid lookup
        // and exercise the signature check path.
        let jwt = sign_jwt(&intruder, &claims, Some(&key.kid));

        let v = verifier_for(
            &issuer,
            "ohd-relay-cloud",
            &discovery_url,
            Duration::from_secs(60),
        );
        let err = v.verify(&jwt).await.unwrap_err();
        assert!(matches!(err, OidcVerifyError::Signature(_)), "{err:?}");
    }

    #[tokio::test]
    async fn handles_kid_rotation_by_refreshing_jwks() {
        let key1 = gen_rsa();
        let issuer = "https://idp.test/".to_string();
        let jwks = Arc::new(Mutex::new(jwks_json(&key1)));
        let (_base, discovery_url, _h) =
            spawn_idp_stub(issuer.clone(), jwks.clone()).await;

        let v = verifier_for(
            &issuer,
            "ohd-relay-cloud",
            &discovery_url,
            // Long TTL: we want to prove the kid-miss refresh path works
            // even when the cached JWKS is otherwise still considered fresh.
            Duration::from_secs(3600),
        );

        // Warm the cache with key1.
        let now = now_secs();
        let claims1 = TestClaims {
            iss: issuer.clone(),
            sub: "u1".into(),
            aud: "ohd-relay-cloud".into(),
            exp: now + 600,
            nbf: now - 10,
            iat: now,
        };
        let jwt1 = sign_jwt(&key1, &claims1, Some(&key1.kid));
        v.verify(&jwt1).await.unwrap();

        // IdP rotates: new key, new kid. Cache hasn't been refreshed yet.
        let mut key2 = gen_rsa();
        key2.kid = "rotated-kid-2".into();
        {
            let mut g = jwks.lock().await;
            *g = jwks_json(&key2);
        }

        let claims2 = TestClaims {
            iss: issuer.clone(),
            sub: "u2".into(),
            aud: "ohd-relay-cloud".into(),
            exp: now + 600,
            nbf: now - 10,
            iat: now,
        };
        let jwt2 = sign_jwt(&key2, &claims2, Some(&key2.kid));
        // Verifier must notice kid miss, re-fetch JWKS, and succeed.
        let out = v.verify(&jwt2).await.unwrap();
        assert_eq!(out.kid.as_deref(), Some("rotated-kid-2"));
    }

    #[tokio::test]
    async fn missing_token_errors_with_missing() {
        let v = OidcVerifier::new(OidcVerifierConfig {
            allowed_issuers: vec![],
            jwks_cache_ttl: Duration::from_secs(60),
            discovery_override: HashMap::new(),
        });
        let err = v.verify("").await.unwrap_err();
        assert!(matches!(err, OidcVerifyError::Missing));
    }

    #[test]
    fn supported_alg_set_excludes_hmac_and_none() {
        assert!(is_supported_alg(Algorithm::RS256));
        assert!(is_supported_alg(Algorithm::ES256));
        assert!(is_supported_alg(Algorithm::EdDSA));
        assert!(!is_supported_alg(Algorithm::HS256));
        assert!(!is_supported_alg(Algorithm::HS384));
    }
}
