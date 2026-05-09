//! FCM HTTP v1 client.
//!
//! # Wire shape
//!
//! ```text
//! POST https://fcm.googleapis.com/v1/projects/<project_id>/messages:send
//! Authorization: Bearer <oauth2_access_token>
//! Content-Type: application/json
//!
//! {
//!   "message": {
//!     "token": "<device_fcm_token>",
//!     "data": { "category": "tunnel_wake", "ref_ulid": "<rendezvous_id>" },
//!     "android": { "priority": "high" }
//!   }
//! }
//! ```
//!
//! Per `spec/notifications.md`:
//! - Data-only message (no `notification` key) so Connect mobile gets the
//!   wake without rendering an OS-level surface.
//! - `android.priority = high` so Doze / App Standby don't defer the wake.
//! - No PHI in the payload — Connect re-fetches details over OHDC after wake.
//!
//! # OAuth2 service-account flow
//!
//! FCM HTTP v1 requires a Google OAuth2 access token derived from a service
//! account JSON file. Rather than pull all of `gcp_auth`'s dep tree (which
//! drags in tonic + a full Google SDK), we mint the bearer ourselves:
//!
//! 1. Read the service-account JSON (PEM private key + client_email).
//! 2. Build a JWT with header `{alg:RS256, typ:JWT, kid:<private_key_id>}`,
//!    claims `{iss, scope, aud:"https://oauth2.googleapis.com/token",
//!    exp:now+3600, iat:now}`.
//! 3. POST `grant_type=urn:ietf:params:oauth:grant-type:jwt-bearer` +
//!    `assertion=<jwt>` to `https://oauth2.googleapis.com/token`.
//! 4. Cache the returned `access_token` until ~5 minutes before its
//!    `expires_in`.
//!
//! This is ~80 lines of `jsonwebtoken` + `reqwest` and avoids ~50 transitive
//! deps. The token-cache lock is uncontended in the relay's typical usage
//! pattern (one push per consumer-attach-while-storage-asleep, single-digit
//! per second at peak).
//!
//! # Retry policy
//!
//! - 401 → invalidate cached bearer, refresh once, retry once. If 401
//!   repeats, surface as `PushError::Auth`.
//! - 404 / `UNREGISTERED` → `PushError::InvalidToken`. Caller marks the
//!   token dead in the registration table.
//! - 429 / 5xx → exponential backoff with `Retry-After` honored when
//!   present. 3 attempts total.
//! - Anything else → `PushError::Provider { status, body }`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, warn};

use super::{PushClient, PushError, TunnelWakePayload};
use crate::state::PushToken;

/// Configurable FCM client settings.
#[derive(Debug, Clone)]
pub struct FcmConfig {
    /// Google Cloud project ID, e.g. `"ohd-cloud"`. Substituted into
    /// the `messages:send` URL path.
    pub project_id: String,
    /// Filesystem path to the service-account JSON. Loaded once at
    /// construction; the client caches the parsed key.
    pub service_account_path: PathBuf,
    /// Override for the FCM messages:send endpoint. In production
    /// always `https://fcm.googleapis.com`. Tests point this at a local
    /// stub.
    pub fcm_base_url: Option<String>,
    /// Override for the OAuth2 token endpoint. In production always
    /// `https://oauth2.googleapis.com/token`. Tests point this at a local
    /// stub.
    pub token_base_url: Option<String>,
}

impl FcmConfig {
    /// Build with the production endpoints.
    pub fn production(project_id: impl Into<String>, service_account_path: PathBuf) -> Self {
        Self {
            project_id: project_id.into(),
            service_account_path,
            fcm_base_url: None,
            token_base_url: None,
        }
    }

    fn fcm_url(&self) -> String {
        let base = self
            .fcm_base_url
            .as_deref()
            .unwrap_or("https://fcm.googleapis.com");
        format!("{base}/v1/projects/{}/messages:send", self.project_id)
    }

    fn token_url(&self) -> &str {
        self.token_base_url
            .as_deref()
            .unwrap_or("https://oauth2.googleapis.com/token")
    }
}

/// Service-account JSON shape (subset; ignores fields we don't use).
#[derive(Debug, Deserialize)]
struct ServiceAccountJson {
    client_email: String,
    private_key: String,
    private_key_id: Option<String>,
    token_uri: Option<String>,
}

#[derive(Clone)]
struct ServiceAccount {
    client_email: String,
    encoding_key: Arc<EncodingKey>,
    private_key_id: Option<String>,
    token_uri: String,
}

impl std::fmt::Debug for ServiceAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceAccount")
            .field("client_email", &self.client_email)
            .field("private_key_id", &self.private_key_id)
            .field("token_uri", &self.token_uri)
            .finish()
    }
}

impl ServiceAccount {
    fn load(path: &std::path::Path, fallback_token_uri: &str) -> Result<Self, PushError> {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| PushError::Auth(format!("read service account: {e}")))?;
        let parsed: ServiceAccountJson = serde_json::from_str(&raw)
            .map_err(|e| PushError::Auth(format!("parse service account: {e}")))?;
        let key = EncodingKey::from_rsa_pem(parsed.private_key.as_bytes())
            .map_err(|e| PushError::Auth(format!("rsa key parse: {e}")))?;
        Ok(Self {
            client_email: parsed.client_email,
            encoding_key: Arc::new(key),
            private_key_id: parsed.private_key_id,
            token_uri: parsed.token_uri.unwrap_or_else(|| fallback_token_uri.to_string()),
        })
    }
}

/// JWT claims for the OAuth2 jwt-bearer flow.
#[derive(Debug, Serialize)]
struct JwtClaims<'a> {
    iss: &'a str,
    scope: &'a str,
    aud: &'a str,
    exp: u64,
    iat: u64,
}

#[derive(Debug, Deserialize)]
struct OauthTokenResponse {
    access_token: String,
    /// Lifetime in seconds (typically 3600).
    #[serde(default)]
    expires_in: u64,
}

#[derive(Debug, Clone)]
struct CachedToken {
    access_token: String,
    /// Wall-clock UNIX-ms when this token expires (refresh ~5 min before).
    expires_at_ms: u64,
}

impl CachedToken {
    fn is_fresh(&self, now_ms: u64) -> bool {
        // Refresh 5 minutes before actual expiry.
        now_ms + 300_000 < self.expires_at_ms
    }
}

const FCM_OAUTH_SCOPE: &str = "https://www.googleapis.com/auth/firebase.messaging";

/// Real FCM HTTP v1 push client. Cheap to clone (`Arc` interior).
#[derive(Clone)]
pub struct FcmPushClient {
    cfg: Arc<FcmConfig>,
    sa: Arc<ServiceAccount>,
    http: HttpClient,
    cached: Arc<RwLock<Option<CachedToken>>>,
}

impl std::fmt::Debug for FcmPushClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FcmPushClient")
            .field("project_id", &self.cfg.project_id)
            .field("client_email", &self.sa.client_email)
            .finish()
    }
}

impl FcmPushClient {
    /// Build a new FCM client. Reads the service-account JSON from disk and
    /// pre-parses the RSA key. Returns `Err` if the JSON is malformed or
    /// the file is unreadable.
    pub fn new(cfg: FcmConfig) -> Result<Self, PushError> {
        let token_uri = cfg.token_url().to_string();
        let sa = ServiceAccount::load(&cfg.service_account_path, &token_uri)?;
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .map_err(|e| PushError::Transport(format!("build http: {e}")))?;
        Ok(Self {
            cfg: Arc::new(cfg),
            sa: Arc::new(sa),
            http,
            cached: Arc::new(RwLock::new(None)),
        })
    }

    /// For tests: build with an injected `reqwest::Client` (e.g. pointing
    /// at a local stub).
    pub fn with_http(cfg: FcmConfig, http: HttpClient) -> Result<Self, PushError> {
        let token_uri = cfg.token_url().to_string();
        let sa = ServiceAccount::load(&cfg.service_account_path, &token_uri)?;
        Ok(Self {
            cfg: Arc::new(cfg),
            sa: Arc::new(sa),
            http,
            cached: Arc::new(RwLock::new(None)),
        })
    }

    /// Fetch (or use cached) Google OAuth2 access token.
    async fn access_token(&self) -> Result<String, PushError> {
        // Fast path: cached and fresh.
        {
            let r = self.cached.read().await;
            if let Some(t) = &*r {
                if t.is_fresh(now_ms()) {
                    return Ok(t.access_token.clone());
                }
            }
        }

        // Slow path: mint a new JWT, exchange for an access token.
        let now_secs = now_ms() / 1000;
        let claims = JwtClaims {
            iss: &self.sa.client_email,
            scope: FCM_OAUTH_SCOPE,
            aud: &self.sa.token_uri,
            exp: now_secs + 3600,
            iat: now_secs,
        };
        let mut header = Header::new(Algorithm::RS256);
        if let Some(kid) = &self.sa.private_key_id {
            header.kid = Some(kid.clone());
        }
        let assertion = jsonwebtoken::encode(&header, &claims, &self.sa.encoding_key)
            .map_err(|e| PushError::Auth(format!("jwt encode: {e}")))?;

        let resp = self
            .http
            .post(&self.sa.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|e| PushError::Transport(format!("oauth post: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            return Err(PushError::Auth(format!("oauth {status}: {body}")));
        }

        let parsed: OauthTokenResponse = resp
            .json()
            .await
            .map_err(|e| PushError::Auth(format!("oauth parse: {e}")))?;
        let expires_in = if parsed.expires_in == 0 {
            3600
        } else {
            parsed.expires_in
        };
        let cached = CachedToken {
            access_token: parsed.access_token.clone(),
            expires_at_ms: now_ms() + expires_in * 1000,
        };
        let mut w = self.cached.write().await;
        *w = Some(cached);
        Ok(parsed.access_token)
    }

    /// Force-clear the cached bearer (e.g. after a 401 from FCM).
    async fn invalidate_token(&self) {
        let mut w = self.cached.write().await;
        *w = None;
    }
}

#[derive(Debug, Serialize)]
struct AndroidConfig<'a> {
    priority: &'a str,
}

#[derive(Debug, Serialize)]
struct ApnsAlert<'a> {
    headers: ApnsHeaders<'a>,
}

#[derive(Debug, Serialize)]
struct ApnsHeaders<'a> {
    #[serde(rename = "apns-priority")]
    apns_priority: &'a str,
    #[serde(rename = "apns-push-type")]
    apns_push_type: &'a str,
}

#[derive(Debug, Serialize)]
struct FcmMessage<'a> {
    token: &'a str,
    data: serde_json::Value,
    android: AndroidConfig<'a>,
    /// Mirror APNs hints so iOS clients that for any reason are routed
    /// through FCM (rare: developers mis-tagging tokens) still see a
    /// background-priority push.
    apns: ApnsAlert<'a>,
}

#[derive(Debug, Serialize)]
struct FcmEnvelope<'a> {
    message: FcmMessage<'a>,
}

#[async_trait]
impl PushClient for FcmPushClient {
    async fn wake(&self, rendezvous_id: &str, token: &PushToken) -> Result<(), PushError> {
        let device_token = match token {
            PushToken::Fcm(t) => t,
            _ => return Err(PushError::UnsupportedTokenType),
        };

        let payload = TunnelWakePayload {
            category: "tunnel_wake",
            ref_ulid: rendezvous_id,
        };
        // FCM requires `data` values to be strings; serialize each field.
        let data = serde_json::json!({
            "category": payload.category,
            "ref_ulid": payload.ref_ulid,
        });
        let envelope = FcmEnvelope {
            message: FcmMessage {
                token: device_token,
                data,
                android: AndroidConfig { priority: "high" },
                apns: ApnsAlert {
                    headers: ApnsHeaders {
                        apns_priority: "10",
                        apns_push_type: "background",
                    },
                },
            },
        };
        let body = serde_json::to_vec(&envelope)
            .map_err(|e| PushError::Transport(format!("serialize: {e}")))?;
        let url = self.cfg.fcm_url();

        let mut last_err: Option<PushError> = None;
        for attempt in 0..3u32 {
            let bearer = self.access_token().await?;
            let resp = self
                .http
                .post(&url)
                .bearer_auth(&bearer)
                .header("Content-Type", "application/json")
                .body(body.clone())
                .send()
                .await;

            match resp {
                Ok(r) => {
                    let status = r.status();
                    if status.is_success() {
                        debug!(
                            target: "ohd_relay::push::fcm",
                            rendezvous_id,
                            "FCM push delivered"
                        );
                        return Ok(());
                    }
                    let code = status.as_u16();
                    let retry_after = retry_after_ms(r.headers());
                    let text = r.text().await.unwrap_or_default();

                    if code == 401 || code == 403 {
                        // Stale bearer: refresh + retry once total.
                        self.invalidate_token().await;
                        if attempt == 0 {
                            last_err = Some(PushError::Auth(format!("auth {code}: {text}")));
                            continue;
                        }
                        return Err(PushError::Auth(format!("auth {code}: {text}")));
                    }
                    if code == 404 || text.contains("UNREGISTERED")
                        || text.contains("INVALID_ARGUMENT")
                    {
                        return Err(PushError::InvalidToken(format!("{code}: {text}")));
                    }
                    if code == 429 || (500..600).contains(&code) {
                        last_err = Some(PushError::Provider {
                            status: code,
                            body: text,
                        });
                        if attempt < 2 {
                            let backoff =
                                retry_after.unwrap_or_else(|| backoff_for_attempt(attempt));
                            warn!(
                                target: "ohd_relay::push::fcm",
                                rendezvous_id,
                                status = code,
                                backoff_ms = backoff.as_millis() as u64,
                                "FCM transient; retrying"
                            );
                            tokio::time::sleep(backoff).await;
                            continue;
                        }
                        return Err(last_err.unwrap());
                    }
                    return Err(PushError::Provider {
                        status: code,
                        body: text,
                    });
                }
                Err(e) => {
                    last_err = Some(PushError::Transport(format!("fcm post: {e}")));
                    if attempt < 2 {
                        tokio::time::sleep(backoff_for_attempt(attempt)).await;
                        continue;
                    }
                    return Err(last_err.unwrap());
                }
            }
        }
        Err(last_err.unwrap_or_else(|| PushError::Transport("fcm: retries exhausted".into())))
    }
}

fn backoff_for_attempt(attempt: u32) -> Duration {
    // 250ms, 1s, 4s.
    let ms = 250u64 << (attempt * 2);
    Duration::from_millis(ms.min(4_000))
}

fn retry_after_ms(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_url_substitutes_project_id() {
        let cfg = FcmConfig::production("ohd-cloud", PathBuf::from("/dev/null"));
        assert_eq!(
            cfg.fcm_url(),
            "https://fcm.googleapis.com/v1/projects/ohd-cloud/messages:send"
        );
    }

    #[test]
    fn config_url_uses_override_for_tests() {
        let cfg = FcmConfig {
            project_id: "p".into(),
            service_account_path: PathBuf::from("/dev/null"),
            fcm_base_url: Some("http://127.0.0.1:9999".into()),
            token_base_url: None,
        };
        assert_eq!(
            cfg.fcm_url(),
            "http://127.0.0.1:9999/v1/projects/p/messages:send"
        );
    }

    #[test]
    fn service_account_json_parses_email_and_key_id() {
        // Key bytes don't need to be a valid RSA PEM for this assertion —
        // we only check the JSON-shape parser. The `EncodingKey::from_rsa_pem`
        // path is exercised in integration tests gated on real creds.
        let raw = serde_json::json!({
            "type": "service_account",
            "project_id": "test-project",
            "private_key_id": "kid-test",
            "private_key": "not-a-real-key",
            "client_email": "sa@test-project.iam.gserviceaccount.com",
            "token_uri": "https://oauth2.googleapis.com/token"
        })
        .to_string();
        let parsed: ServiceAccountJson = serde_json::from_str(&raw).unwrap();
        assert_eq!(
            parsed.client_email,
            "sa@test-project.iam.gserviceaccount.com"
        );
        assert_eq!(parsed.private_key_id.as_deref(), Some("kid-test"));
    }

    #[test]
    fn cached_token_freshness_window() {
        let now = now_ms();
        let stale = CachedToken {
            access_token: "x".into(),
            expires_at_ms: now + 60_000, // expires in 1 min — within 5-min refresh window
        };
        assert!(!stale.is_fresh(now));
        let fresh = CachedToken {
            access_token: "x".into(),
            expires_at_ms: now + 600_000, // expires in 10 min
        };
        assert!(fresh.is_fresh(now));
    }

    #[test]
    fn fcm_envelope_serialization_matches_spec() {
        let env = FcmEnvelope {
            message: FcmMessage {
                token: "tok",
                data: serde_json::json!({"category":"tunnel_wake","ref_ulid":"abc"}),
                android: AndroidConfig { priority: "high" },
                apns: ApnsAlert {
                    headers: ApnsHeaders {
                        apns_priority: "10",
                        apns_push_type: "background",
                    },
                },
            },
        };
        let json = serde_json::to_value(&env).unwrap();
        assert_eq!(json["message"]["token"], "tok");
        assert_eq!(json["message"]["data"]["category"], "tunnel_wake");
        assert_eq!(json["message"]["data"]["ref_ulid"], "abc");
        assert_eq!(json["message"]["android"]["priority"], "high");
        assert_eq!(
            json["message"]["apns"]["headers"]["apns-push-type"],
            "background"
        );
    }

    #[test]
    fn backoff_grows_with_attempt() {
        let a0 = backoff_for_attempt(0);
        let a1 = backoff_for_attempt(1);
        let a2 = backoff_for_attempt(2);
        assert!(a0 < a1);
        assert!(a1 < a2 || a2 == Duration::from_millis(4_000));
    }
}
