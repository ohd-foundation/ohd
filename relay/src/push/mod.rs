//! Push-wake clients.
//!
//! When a consumer attaches but `current_tunnel_endpoint` is `None`, the
//! relay sends a silent push to wake the storage device. Per
//! `spec/notifications.md`, the payload is data-only:
//!
//! ```json
//! { "category": "tunnel_wake", "ref_ulid": "<rendezvous_id>" }
//! ```
//!
//! No PHI, ever. The wake path is the only time the relay touches push
//! providers.
//!
//! ## Module layout
//!
//! - [`PushClient`] / [`PushDispatcher`] — the trait + the per-token-type
//!   router. Used by `src/server.rs` via `state.push.wake(...)`.
//! - [`fcm::FcmPushClient`] — FCM HTTP v1 client. OAuth2 service-account
//!   bearer minted in-process with `jsonwebtoken`; POSTs JSON to
//!   `https://fcm.googleapis.com/v1/projects/<id>/messages:send`. Honors
//!   `Retry-After` on 429/503; exponential backoff up to 3 attempts.
//! - [`apns::ApnsPushClient`] — APNs HTTP/2 client. ES256 JWT with the
//!   team-id + key-id claims; POSTs `{"aps":{"content-available":1}}` (or
//!   loud critical-alert in authority mode) to
//!   `api.push.apple.com/3/device/<token>`. JWT is regenerated every
//!   ~50 minutes.
//!
//! ## Test transport
//!
//! Both clients accept a `reqwest::Client` injected from the outside, so
//! unit tests swap in a `wiremock`-style stub or just point the endpoint
//! at a local httptest server. Real-credential integration tests live
//! under `tests/` and skip themselves if the env vars aren't present.

use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use tracing::debug;

use crate::state::PushToken;

pub mod apns;
pub mod fcm;

pub use apns::{ApnsConfig, ApnsEnvironment, ApnsPushClient, ApnsUrgency};
pub use fcm::{FcmConfig, FcmPushClient};

/// How long the relay waits for storage to reconnect after a push-wake.
pub const WAKE_DEADLINE: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum PushError {
    #[error("push request failed: {0}")]
    Transport(String),
    #[error("push provider rejected token: {0}")]
    InvalidToken(String),
    #[error("push not configured for token type")]
    UnsupportedTokenType,
    #[error("push auth (oauth2 / jwt) failed: {0}")]
    Auth(String),
    #[error("push provider returned {status}: {body}")]
    Provider { status: u16, body: String },
}

/// Payload shape for tunnel-wake. Mirrors `spec/notifications.md`:
/// `{ "category": "tunnel_wake", "ref_ulid": "<rendezvous_id>" }`.
#[derive(Debug, Serialize)]
pub struct TunnelWakePayload<'a> {
    pub category: &'a str,
    pub ref_ulid: &'a str,
}

#[async_trait]
pub trait PushClient: Send + Sync {
    async fn wake(&self, rendezvous_id: &str, token: &PushToken) -> Result<(), PushError>;
}

// ---------------------------------------------------------------------------
// Dispatcher: picks per-platform client
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct PushDispatcher {
    pub fcm: Option<FcmPushClient>,
    pub apns: Option<ApnsPushClient>,
}

impl PushDispatcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_fcm(mut self, client: FcmPushClient) -> Self {
        self.fcm = Some(client);
        self
    }

    pub fn with_apns(mut self, client: ApnsPushClient) -> Self {
        self.apns = Some(client);
        self
    }
}

#[async_trait]
impl PushClient for PushDispatcher {
    async fn wake(&self, rendezvous_id: &str, token: &PushToken) -> Result<(), PushError> {
        debug!(
            target: "ohd_relay::push",
            kind = token.platform(),
            rendezvous_id = %rendezvous_id,
            "dispatching push-wake"
        );
        match token {
            PushToken::Fcm(_) => match &self.fcm {
                Some(c) => c.wake(rendezvous_id, token).await,
                None => Err(PushError::UnsupportedTokenType),
            },
            PushToken::Apns(_) => match &self.apns {
                Some(c) => c.wake(rendezvous_id, token).await,
                None => Err(PushError::UnsupportedTokenType),
            },
            // WebPush + Email do not participate in tunnel-wake; storage
            // doesn't run in browser tabs, and email can't wake a phone
            // immediately.
            _ => Err(PushError::UnsupportedTokenType),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dispatcher_rejects_unsupported_token_types() {
        let d = PushDispatcher::new();
        let r = d.wake("rzv", &PushToken::Email("x@y".into())).await;
        assert!(matches!(r, Err(PushError::UnsupportedTokenType)));
    }

    #[tokio::test]
    async fn dispatcher_rejects_when_backend_not_configured() {
        let d = PushDispatcher::new();
        let r = d.wake("rzv", &PushToken::Fcm("a".into())).await;
        assert!(matches!(r, Err(PushError::UnsupportedTokenType)));
        let r = d.wake("rzv", &PushToken::Apns("a".into())).await;
        assert!(matches!(r, Err(PushError::UnsupportedTokenType)));
    }
}
