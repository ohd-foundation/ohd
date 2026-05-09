//! APNs HTTP/2 client.
//!
//! # Wire shape
//!
//! ```text
//! POST https://api.push.apple.com/3/device/<device_token>
//! Authorization: bearer <jwt>
//! apns-topic: <bundle_id>
//! apns-push-type: background        (alert in authority/Critical-Alert mode)
//! apns-priority: 5                  (10 in authority mode)
//! apns-id: <uuid-v4>                (idempotency / dedupe)
//! Content-Type: application/json
//!
//! { "aps": { "content-available": 1 } }
//! ```
//!
//! Per `spec/notifications.md`:
//! - Background (silent) push for tunnel-wake. iOS routes it to the
//!   notification-service-extension which decides what (if anything) to
//!   surface.
//! - For emergency-authority mode, [`ApnsUrgency::Critical`] flips the
//!   headers to `apns-push-type: alert` + `apns-priority: 10` and the body
//!   to `{"aps":{"alert":{"title":...},"sound":"critical","interruption-level":"critical"}}`.
//!   This requires Apple's Critical Alert entitlement on the OHD Connect
//!   bundle — Apple-issued, separate from this codebase. Without the
//!   entitlement Apple silently downgrades to a normal alert.
//! - `apns-priority: 10` is "deliver immediately"; OS budget rules apply.
//!
//! # JWT auth (token-based)
//!
//! Apple supports both cert-based and token-based auth; we use token. A
//! single `.p8` file holds an EC P-256 private key issued by Apple. The
//! relay builds an ES256 JWT with claims `{iss: team_id, iat: now}` and
//! header `{alg:ES256, kid: key_id}`. JWT lifetime is up to 1 hour per
//! Apple's docs; we regenerate every 50 minutes.
//!
//! # Retry policy
//!
//! Same shape as FCM:
//! - 400 with `BadDeviceToken` / 410 → `PushError::InvalidToken`.
//! - 429 / 5xx → exponential backoff, 3 attempts, honor APNs's
//!   per-connection retry hints.
//! - 401 / `ExpiredProviderToken` → invalidate cached JWT, refresh, retry.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use rand::RngCore;
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing::{debug, warn};

use super::{PushClient, PushError, TunnelWakePayload};
use crate::state::PushToken;

/// APNs environment selector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApnsEnvironment {
    Production,
    Sandbox,
}

impl ApnsEnvironment {
    fn host(self) -> &'static str {
        match self {
            Self::Production => "https://api.push.apple.com",
            Self::Sandbox => "https://api.development.push.apple.com",
        }
    }
}

impl Default for ApnsEnvironment {
    fn default() -> Self {
        Self::Production
    }
}

/// Push urgency. Default is silent/background; Critical is emergency-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApnsUrgency {
    #[default]
    Background,
    /// iOS Critical Alert. Requires Apple's Critical Alert entitlement on
    /// the bundle — without it Apple downgrades to normal alert priority.
    /// Used by the `emergency` push category, NOT by tunnel-wake.
    Critical,
}

#[derive(Debug, Clone)]
pub struct ApnsConfig {
    pub team_id: String,
    pub key_id: String,
    /// Path to the Apple-issued `.p8` private key. Loaded once at
    /// construction.
    pub key_path: PathBuf,
    /// Bundle ID of the OHD Connect iOS app, e.g. `"org.ohd.connect"`.
    /// Sent as the `apns-topic` header.
    pub bundle_id: String,
    pub environment: ApnsEnvironment,
    /// Override for the APNs base URL. Tests stub a local server.
    pub override_base_url: Option<String>,
}

impl ApnsConfig {
    fn base_url(&self) -> String {
        if let Some(b) = &self.override_base_url {
            return b.clone();
        }
        self.environment.host().to_string()
    }
}

#[derive(Debug, Serialize)]
struct JwtClaims<'a> {
    iss: &'a str,
    iat: u64,
}

#[derive(Debug, Clone)]
struct CachedJwt {
    token: String,
    /// Wall-clock UNIX-ms when this JWT was minted; refresh after 50 min.
    issued_at_ms: u64,
}

impl CachedJwt {
    fn is_fresh(&self, now_ms: u64) -> bool {
        // Apple says max 1h; refresh 10 min early.
        now_ms < self.issued_at_ms + 50 * 60 * 1000
    }
}

#[derive(Clone)]
pub struct ApnsPushClient {
    cfg: Arc<ApnsConfig>,
    encoding_key: Arc<EncodingKey>,
    http: HttpClient,
    cached_jwt: Arc<RwLock<Option<CachedJwt>>>,
}

impl std::fmt::Debug for ApnsPushClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApnsPushClient")
            .field("team_id", &self.cfg.team_id)
            .field("key_id", &self.cfg.key_id)
            .field("bundle_id", &self.cfg.bundle_id)
            .field("environment", &self.cfg.environment)
            .finish()
    }
}

impl ApnsPushClient {
    pub fn new(cfg: ApnsConfig) -> Result<Self, PushError> {
        let pem = std::fs::read(&cfg.key_path)
            .map_err(|e| PushError::Auth(format!("read p8: {e}")))?;
        // Apple's .p8 is a PKCS#8-PEM-encoded EC P-256 private key.
        let key = EncodingKey::from_ec_pem(&pem)
            .map_err(|e| PushError::Auth(format!("p8 parse: {e}")))?;
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(15))
            .http2_prior_knowledge()
            .build()
            .map_err(|e| PushError::Transport(format!("build http: {e}")))?;
        Ok(Self {
            cfg: Arc::new(cfg),
            encoding_key: Arc::new(key),
            http,
            cached_jwt: Arc::new(RwLock::new(None)),
        })
    }

    /// For tests: build with a custom HTTP client (e.g. without
    /// `http2_prior_knowledge` so we can speak h2 over plain TCP to a stub
    /// or hit an HTTP/1 mock server).
    pub fn with_http(cfg: ApnsConfig, http: HttpClient) -> Result<Self, PushError> {
        let pem = std::fs::read(&cfg.key_path)
            .map_err(|e| PushError::Auth(format!("read p8: {e}")))?;
        let key = EncodingKey::from_ec_pem(&pem)
            .map_err(|e| PushError::Auth(format!("p8 parse: {e}")))?;
        Ok(Self {
            cfg: Arc::new(cfg),
            encoding_key: Arc::new(key),
            http,
            cached_jwt: Arc::new(RwLock::new(None)),
        })
    }

    async fn jwt(&self) -> Result<String, PushError> {
        {
            let r = self.cached_jwt.read().await;
            if let Some(j) = &*r {
                if j.is_fresh(now_ms()) {
                    return Ok(j.token.clone());
                }
            }
        }
        let now_secs = now_ms() / 1000;
        let claims = JwtClaims {
            iss: &self.cfg.team_id,
            iat: now_secs,
        };
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.cfg.key_id.clone());
        let token = jsonwebtoken::encode(&header, &claims, &self.encoding_key)
            .map_err(|e| PushError::Auth(format!("apns jwt encode: {e}")))?;
        let cached = CachedJwt {
            token: token.clone(),
            issued_at_ms: now_ms(),
        };
        let mut w = self.cached_jwt.write().await;
        *w = Some(cached);
        Ok(token)
    }

    async fn invalidate_jwt(&self) {
        let mut w = self.cached_jwt.write().await;
        *w = None;
    }

    /// Send a push to the given device with arbitrary urgency + body. The
    /// public `wake` calls this with `Background` + tunnel-wake payload;
    /// the emergency entry point on `src/server.rs` calls it with
    /// `Critical` + emergency payload.
    pub async fn send(
        &self,
        device_token: &str,
        urgency: ApnsUrgency,
        body: &serde_json::Value,
    ) -> Result<(), PushError> {
        let url = format!("{}/3/device/{}", self.cfg.base_url(), device_token);
        let bytes = serde_json::to_vec(body)
            .map_err(|e| PushError::Transport(format!("serialize: {e}")))?;

        let push_type = match urgency {
            ApnsUrgency::Background => "background",
            ApnsUrgency::Critical => "alert",
        };
        let priority = match urgency {
            ApnsUrgency::Background => "5",
            ApnsUrgency::Critical => "10",
        };
        let apns_id = uuid_v4_string();

        let mut last_err: Option<PushError> = None;
        for attempt in 0..3u32 {
            let jwt = self.jwt().await?;
            let resp = self
                .http
                .post(&url)
                .header("authorization", format!("bearer {jwt}"))
                .header("apns-topic", &self.cfg.bundle_id)
                .header("apns-push-type", push_type)
                .header("apns-priority", priority)
                .header("apns-id", &apns_id)
                .header("Content-Type", "application/json")
                .body(bytes.clone())
                .send()
                .await;

            match resp {
                Ok(r) => {
                    let status = r.status().as_u16();
                    if (200..300).contains(&status) {
                        debug!(
                            target: "ohd_relay::push::apns",
                            apns_id = %apns_id,
                            "APNs push delivered"
                        );
                        return Ok(());
                    }
                    let text = r.text().await.unwrap_or_default();
                    if status == 400 && text.contains("BadDeviceToken") {
                        return Err(PushError::InvalidToken(format!("400: {text}")));
                    }
                    if status == 410 {
                        return Err(PushError::InvalidToken(format!(
                            "Unregistered (410): {text}"
                        )));
                    }
                    if status == 403 && text.contains("ExpiredProviderToken") {
                        self.invalidate_jwt().await;
                        if attempt == 0 {
                            last_err = Some(PushError::Auth(format!("apns 403: {text}")));
                            continue;
                        }
                        return Err(PushError::Auth(format!("apns 403: {text}")));
                    }
                    if status == 429 || (500..600).contains(&status) {
                        last_err = Some(PushError::Provider {
                            status,
                            body: text,
                        });
                        if attempt < 2 {
                            let backoff = backoff_for_attempt(attempt);
                            warn!(
                                target: "ohd_relay::push::apns",
                                apns_id = %apns_id,
                                status,
                                backoff_ms = backoff.as_millis() as u64,
                                "APNs transient; retrying"
                            );
                            tokio::time::sleep(backoff).await;
                            continue;
                        }
                        return Err(last_err.unwrap());
                    }
                    return Err(PushError::Provider {
                        status,
                        body: text,
                    });
                }
                Err(e) => {
                    last_err = Some(PushError::Transport(format!("apns post: {e}")));
                    if attempt < 2 {
                        tokio::time::sleep(backoff_for_attempt(attempt)).await;
                        continue;
                    }
                    return Err(last_err.unwrap());
                }
            }
        }
        Err(last_err.unwrap_or_else(|| PushError::Transport("apns: retries exhausted".into())))
    }
}

#[derive(Debug, Serialize)]
struct ApsBackground {
    #[serde(rename = "content-available")]
    content_available: u8,
}

#[derive(Debug, Serialize)]
struct ApsBackgroundEnvelope {
    aps: ApsBackground,
    /// Mirror the FCM data block so iOS receivers see the same shape.
    #[serde(flatten)]
    data: serde_json::Value,
}

#[async_trait]
impl PushClient for ApnsPushClient {
    async fn wake(&self, rendezvous_id: &str, token: &PushToken) -> Result<(), PushError> {
        let device_token = match token {
            PushToken::Apns(t) => t,
            _ => return Err(PushError::UnsupportedTokenType),
        };
        let payload = TunnelWakePayload {
            category: "tunnel_wake",
            ref_ulid: rendezvous_id,
        };
        let body = ApsBackgroundEnvelope {
            aps: ApsBackground {
                content_available: 1,
            },
            data: serde_json::json!({
                "category": payload.category,
                "ref_ulid": payload.ref_ulid,
            }),
        };
        let body_value = serde_json::to_value(&body)
            .map_err(|e| PushError::Transport(format!("serialize: {e}")))?;
        self.send(device_token, ApnsUrgency::Background, &body_value).await
    }
}

fn backoff_for_attempt(attempt: u32) -> Duration {
    let ms = 250u64 << (attempt * 2);
    Duration::from_millis(ms.min(4_000))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Tiny v4-shaped UUID generator (random 16 bytes formatted as
/// `xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx`). Used for `apns-id` so APNs
/// can dedupe identical requests; we don't need true type-4 entropy
/// guarantees for this use.
fn uuid_v4_string() -> String {
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    // Set version 4 + variant bits per RFC 4122.
    buf[6] = (buf[6] & 0x0F) | 0x40;
    buf[8] = (buf[8] & 0x3F) | 0x80;
    let mut s = String::with_capacity(36);
    for (i, b) in buf.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            s.push('-');
        }
        s.push_str(&format!("{:02x}", b));
    }
    s
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ApnsErrorBody {
    reason: String,
    timestamp: Option<u64>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_picks_environment() {
        assert_eq!(
            ApnsEnvironment::Production.host(),
            "https://api.push.apple.com"
        );
        assert_eq!(
            ApnsEnvironment::Sandbox.host(),
            "https://api.development.push.apple.com"
        );
    }

    #[test]
    fn cached_jwt_freshness_window() {
        let now = now_ms();
        let stale = CachedJwt {
            token: "x".into(),
            issued_at_ms: now - 60 * 60 * 1000, // 1h old
        };
        assert!(!stale.is_fresh(now));
        let fresh = CachedJwt {
            token: "x".into(),
            issued_at_ms: now - 10 * 60 * 1000, // 10 min old
        };
        assert!(fresh.is_fresh(now));
    }

    #[test]
    fn uuid_format_is_36_chars_with_dashes() {
        let u = uuid_v4_string();
        assert_eq!(u.len(), 36);
        assert_eq!(&u[8..9], "-");
        assert_eq!(&u[13..14], "-");
        assert_eq!(&u[18..19], "-");
        assert_eq!(&u[23..24], "-");
    }

    #[test]
    fn aps_envelope_serializes_with_content_available() {
        let env = ApsBackgroundEnvelope {
            aps: ApsBackground {
                content_available: 1,
            },
            data: serde_json::json!({
                "category": "tunnel_wake",
                "ref_ulid": "abc",
            }),
        };
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["aps"]["content-available"], 1);
        assert_eq!(v["category"], "tunnel_wake");
        assert_eq!(v["ref_ulid"], "abc");
    }

    #[test]
    fn config_base_url_uses_override_when_set() {
        let cfg = ApnsConfig {
            team_id: "T".into(),
            key_id: "K".into(),
            key_path: PathBuf::from("/dev/null"),
            bundle_id: "org.ohd.connect".into(),
            environment: ApnsEnvironment::Production,
            override_base_url: Some("http://127.0.0.1:9".into()),
        };
        assert_eq!(cfg.base_url(), "http://127.0.0.1:9");
    }

    #[test]
    fn config_base_url_falls_back_to_environment() {
        let cfg = ApnsConfig {
            team_id: "T".into(),
            key_id: "K".into(),
            key_path: PathBuf::from("/dev/null"),
            bundle_id: "org.ohd.connect".into(),
            environment: ApnsEnvironment::Sandbox,
            override_base_url: None,
        };
        assert_eq!(cfg.base_url(), "https://api.development.push.apple.com");
    }
}
