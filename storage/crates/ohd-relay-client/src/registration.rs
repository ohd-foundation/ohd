//! Relay registration HTTP flow.
//!
//! Implements the storage-side of the relay's registration endpoints
//! documented in `relay/spec/relay-protocol.md` "Storage registration":
//!
//! - `POST {base}/register`   — first-time registration; yields a
//!   `rendezvous_id` + `long_lived_credential`.
//! - `POST {base}/heartbeat`  — registration-level keepalive.
//! - `POST {base}/deregister` — clean farewell; drops the rendezvous record.
//!
//! The wire shape (JSON request/response bodies, status codes) mirrors the
//! relay's `server.rs` handlers exactly. `base` defaults to `/v1` — the
//! path the relay binary actually mounts (`build_router`). The spec doc
//! writes it as `/relay/v1`; the [`RegistrationClient::base_path`] knob
//! lets a caller match either deployment.
//!
//! This module is portable: it depends only on `reqwest` (rustls-TLS) +
//! `serde`, so it cross-compiles for the Android targets.

use serde::{Deserialize, Serialize};

/// Errors from the registration HTTP flow.
#[derive(Debug, thiserror::Error)]
pub enum RegistrationError {
    #[error("http transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("relay rejected request: HTTP {status} {body}")]
    Rejected { status: u16, body: String },
    #[error("malformed relay url: {0}")]
    BadUrl(String),
}

// ---------------------------------------------------------------------------
// Wire types — mirror `relay/src/server.rs`.
// ---------------------------------------------------------------------------

/// Push-token wire enum — matches the relay's `PushTokenWire`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "platform", content = "value")]
pub enum PushToken {
    #[serde(rename = "fcm")]
    Fcm(String),
    #[serde(rename = "apns")]
    Apns(String),
    #[serde(rename = "email")]
    Email(String),
    #[serde(rename = "web_push")]
    WebPush {
        endpoint: String,
        p256dh: String,
        auth: String,
    },
}

/// `POST {base}/register` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    /// Hex-encoded 16-byte user ULID.
    pub user_ulid: String,
    /// Hex-encoded Ed25519 SPKI bytes of the storage identity key.
    pub storage_pubkey_spki_hex: String,
    /// Optional FCM/APNs/email/web-push token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub push_token: Option<PushToken>,
    /// Optional friendly label for the user-visible relay listing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_label: Option<String>,
    /// Optional OIDC `id_token` (compact JWT) for relays that gate
    /// registration behind an issuer allowlist.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
}

/// `POST {base}/register` `201 Created` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub rendezvous_id: String,
    pub rendezvous_url: String,
    pub long_lived_credential: String,
}

/// `POST {base}/heartbeat` and `POST {base}/deregister` request body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialedRequest {
    pub rendezvous_id: String,
    pub long_lived_credential: String,
}

/// `POST {base}/heartbeat` / `POST {base}/deregister` response body.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OkResponse {
    pub ok: bool,
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/// HTTP client for the relay registration endpoints.
///
/// One client can be reused across `register` / `heartbeat` / `deregister`
/// calls — the underlying `reqwest::Client` pools connections.
#[derive(Debug, Clone)]
pub struct RegistrationClient {
    http: reqwest::Client,
    /// Relay origin, e.g. `https://relay.example.com` — no trailing slash.
    origin: String,
    /// Endpoint base path, default `/v1`.
    base_path: String,
}

impl RegistrationClient {
    /// Build a client for the relay at `origin` (e.g.
    /// `https://relay.example.com`). A trailing slash on `origin` is
    /// trimmed. The endpoint base path defaults to `/v1`; override it with
    /// [`RegistrationClient::with_base_path`] for deployments that mount the
    /// endpoints under `/relay/v1` as the spec doc writes them.
    pub fn new(origin: impl Into<String>) -> Result<Self, RegistrationError> {
        let origin = origin.into();
        let origin = origin.trim_end_matches('/').to_string();
        if origin.is_empty() {
            return Err(RegistrationError::BadUrl("empty relay origin".into()));
        }
        let http = reqwest::Client::builder()
            .build()
            .map_err(RegistrationError::Transport)?;
        Ok(Self {
            http,
            origin,
            base_path: "/v1".to_string(),
        })
    }

    /// Override the endpoint base path (default `/v1`).
    pub fn with_base_path(mut self, base_path: impl Into<String>) -> Self {
        let mut p = base_path.into();
        if !p.starts_with('/') {
            p.insert(0, '/');
        }
        let trimmed = p.trim_end_matches('/').to_string();
        self.base_path = trimmed;
        self
    }

    /// Inject a pre-built `reqwest::Client` (e.g. one with a custom
    /// timeout / proxy). Useful for tests and for Android, where the host
    /// app may want to share one HTTP client.
    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    fn endpoint(&self, leaf: &str) -> String {
        format!("{}{}/{}", self.origin, self.base_path, leaf)
    }

    /// `POST {base}/register` — first-time registration.
    pub async fn register(
        &self,
        req: &RegisterRequest,
    ) -> Result<RegisterResponse, RegistrationError> {
        self.post_json("register", req).await
    }

    /// `POST {base}/heartbeat` — registration-level keepalive.
    pub async fn heartbeat(
        &self,
        req: &CredentialedRequest,
    ) -> Result<OkResponse, RegistrationError> {
        self.post_json("heartbeat", req).await
    }

    /// `POST {base}/deregister` — drop the registration.
    pub async fn deregister(
        &self,
        req: &CredentialedRequest,
    ) -> Result<OkResponse, RegistrationError> {
        self.post_json("deregister", req).await
    }

    async fn post_json<B: Serialize, R: for<'de> Deserialize<'de>>(
        &self,
        leaf: &str,
        body: &B,
    ) -> Result<R, RegistrationError> {
        let url = self.endpoint(leaf);
        let resp = self.http.post(&url).json(body).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(RegistrationError::Rejected {
                status: status.as_u16(),
                body,
            });
        }
        let parsed = resp.json::<R>().await?;
        Ok(parsed)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_request_serializes_minimal() {
        let req = RegisterRequest {
            user_ulid: "0123456789abcdef0123456789abcdef".into(),
            storage_pubkey_spki_hex: "aabb".into(),
            push_token: None,
            user_label: None,
            id_token: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        // Optional fields elided when None.
        assert_eq!(json.get("push_token"), None);
        assert_eq!(json.get("user_label"), None);
        assert_eq!(json.get("id_token"), None);
        assert_eq!(
            json["storage_pubkey_spki_hex"].as_str(),
            Some("aabb")
        );
    }

    #[test]
    fn register_request_serializes_with_push_token() {
        let req = RegisterRequest {
            user_ulid: "00".into(),
            storage_pubkey_spki_hex: "11".into(),
            push_token: Some(PushToken::Fcm("tok".into())),
            user_label: Some("Pixel".into()),
            id_token: None,
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["push_token"]["platform"].as_str(), Some("fcm"));
        assert_eq!(json["push_token"]["value"].as_str(), Some("tok"));
        assert_eq!(json["user_label"].as_str(), Some("Pixel"));
    }

    #[test]
    fn register_response_roundtrip() {
        let wire = r#"{
            "rendezvous_id": "abc22charbase32",
            "rendezvous_url": "wss://relay.example.com/v1/tunnel/abc22charbase32",
            "long_lived_credential": "llc_opaque"
        }"#;
        let resp: RegisterResponse = serde_json::from_str(wire).unwrap();
        assert_eq!(resp.rendezvous_id, "abc22charbase32");
        assert_eq!(resp.long_lived_credential, "llc_opaque");
    }

    #[test]
    fn push_token_web_push_roundtrip() {
        let t = PushToken::WebPush {
            endpoint: "https://push.example/e".into(),
            p256dh: "key".into(),
            auth: "secret".into(),
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: PushToken = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }

    #[test]
    fn client_endpoint_default_base() {
        let c = RegistrationClient::new("https://relay.example.com/").unwrap();
        assert_eq!(
            c.endpoint("register"),
            "https://relay.example.com/v1/register"
        );
        assert_eq!(
            c.endpoint("heartbeat"),
            "https://relay.example.com/v1/heartbeat"
        );
    }

    #[test]
    fn client_endpoint_custom_base() {
        let c = RegistrationClient::new("https://relay.example.com")
            .unwrap()
            .with_base_path("relay/v1/");
        assert_eq!(
            c.endpoint("register"),
            "https://relay.example.com/relay/v1/register"
        );
    }

    #[test]
    fn client_rejects_empty_origin() {
        assert!(RegistrationClient::new("").is_err());
        assert!(RegistrationClient::new("/").is_err());
    }
}
