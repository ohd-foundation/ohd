//! Authority cert chain holder + refresh scheduler.
//!
//! The relay holds one active chain at a time. Refresh logic:
//!
//! 1. On startup, attempt to refresh.
//! 2. Cache the resulting chain.
//! 3. A background task wakes every minute, checks if the chain is within
//!    `refresh_window` of expiring; if so, refreshes.
//! 4. New chain is logged to Rekor (soft-fail), then atomically swapped in.
//! 5. On refresh failure: log + retry every 5 minutes; the existing chain
//!    keeps working until its `notAfter` (caller's `is_current` check is
//!    the safety net).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use ed25519_dalek::SigningKey;
use rand::rngs::OsRng;
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::cert_chain::{now_ms, AuthorityCertChain};
use super::fulcio::{FulcioClient, FulcioConfig};
use super::rekor::{RekorClient, RekorConfig};
use super::AuthorityError;

/// User-tunable bits.
#[derive(Debug, Clone)]
pub struct AuthorityStateConfig {
    pub fulcio: FulcioConfig,
    pub rekor: Option<RekorConfig>,
    /// How close to expiry the refresh loop fires. Default: 1h before
    /// expiry.
    pub refresh_window: Duration,
    /// How often the refresh loop wakes. Default: 60s.
    pub poll_interval: Duration,
    /// On failure, how long to wait before retrying. Default: 5min.
    pub retry_backoff: Duration,
    /// Path to a file containing the OIDC ID token. Re-read every refresh
    /// (token-rotation is the deployment system's job).
    pub oidc_id_token_path: PathBuf,
    /// `email` claim value the OIDC token carries. We don't parse the JWT
    /// here; the deployment supplies this alongside the token file.
    /// (Could be derived from the JWT — left explicit for simplicity.)
    pub oidc_email_claim: String,
}

impl Default for AuthorityStateConfig {
    fn default() -> Self {
        Self {
            fulcio: FulcioConfig {
                fulcio_url: "https://fulcio.openhealth-data.org".into(),
                override_signing_cert_url: None,
            },
            rekor: None,
            refresh_window: Duration::from_secs(60 * 60), // 1h
            poll_interval: Duration::from_secs(60),
            retry_backoff: Duration::from_secs(5 * 60),
            oidc_id_token_path: PathBuf::new(),
            oidc_email_claim: String::new(),
        }
    }
}

/// Shared cert-chain holder + refresh client. Cheap to clone (`Arc`
/// interior).
#[derive(Clone)]
pub struct AuthorityState {
    inner: Arc<AuthorityStateInner>,
}

struct AuthorityStateInner {
    cfg: AuthorityStateConfig,
    fulcio: FulcioClient,
    rekor: Option<RekorClient>,
    /// Active chain. `None` until first successful refresh.
    chain: RwLock<Option<AuthorityCertChain>>,
}

impl AuthorityState {
    pub fn new(cfg: AuthorityStateConfig) -> Result<Self, AuthorityError> {
        let fulcio = FulcioClient::new(cfg.fulcio.clone())?;
        let rekor = cfg
            .rekor
            .clone()
            .map(RekorClient::new)
            .transpose()?;
        Ok(Self {
            inner: Arc::new(AuthorityStateInner {
                cfg,
                fulcio,
                rekor,
                chain: RwLock::new(None),
            }),
        })
    }

    /// Get the current chain, if any.
    pub async fn current(&self) -> Option<AuthorityCertChain> {
        self.inner.chain.read().await.clone()
    }

    /// Force a refresh now. Used at startup + by the background loop.
    pub async fn refresh(&self) -> Result<(), AuthorityError> {
        let cfg = &self.inner.cfg;
        let id_token = std::fs::read_to_string(&cfg.oidc_id_token_path)
            .map_err(|e| AuthorityError::Config(format!("read OIDC token: {e}")))?;
        let id_token = id_token.trim().to_string();

        // Mint a fresh keypair per refresh — short-lived per the spec.
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);

        let chain = self
            .inner
            .fulcio
            .refresh_cert(&id_token, &cfg.oidc_email_claim, &signing_key)
            .await?;

        // Log to Rekor (soft-fail).
        if let Some(rekor) = &self.inner.rekor {
            match rekor.submit_cert(&chain.leaf_pem).await {
                Ok(Some(idx)) => {
                    info!(
                        target: "ohd_relay::auth_mode",
                        log_index = idx,
                        "rekor entry submitted"
                    );
                }
                Ok(None) => {
                    // soft-fail mode hit
                }
                Err(e) => {
                    warn!(
                        target: "ohd_relay::auth_mode",
                        error = %e,
                        "rekor submission failed; continuing"
                    );
                }
            }
        }

        let mut w = self.inner.chain.write().await;
        *w = Some(chain);
        info!(
            target: "ohd_relay::auth_mode",
            "authority cert refreshed"
        );
        Ok(())
    }

    /// Returns true if the cached chain is within the refresh window of
    /// expiring (or absent entirely).
    pub async fn needs_refresh(&self) -> bool {
        let r = self.inner.chain.read().await;
        match &*r {
            None => true,
            Some(c) => {
                let until = c.millis_until_expiry(now_ms()) as u128;
                until <= self.inner.cfg.refresh_window.as_millis()
            }
        }
    }
}

/// Run the refresh loop forever. Cancellation is via dropping the
/// `AuthorityState` clone on the caller's side.
pub async fn run_refresh_loop(state: AuthorityState) {
    loop {
        if state.needs_refresh().await {
            match state.refresh().await {
                Ok(()) => {}
                Err(e) => {
                    warn!(
                        target: "ohd_relay::auth_mode",
                        error = %e,
                        "authority cert refresh failed; will retry"
                    );
                    tokio::time::sleep(state.inner.cfg.retry_backoff).await;
                    continue;
                }
            }
        }
        tokio::time::sleep(state.inner.cfg.poll_interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_sane_intervals() {
        let cfg = AuthorityStateConfig::default();
        assert_eq!(cfg.refresh_window, Duration::from_secs(60 * 60));
        assert_eq!(cfg.poll_interval, Duration::from_secs(60));
    }
}
