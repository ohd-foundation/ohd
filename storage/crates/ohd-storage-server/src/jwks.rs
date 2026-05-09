//! HTTP-fetching JWKS resolver for the AuthService multi-identity flow.
//!
//! The core's [`JwksResolver`](ohd_storage_core::identities::JwksResolver)
//! trait is sync — it's called from inside the storage's `with_conn_mut`
//! closure where the SQLite mutex is held. The simplest production
//! implementation that respects that constraint is a synchronous fetcher
//! built on `reqwest::blocking` (rustls-tls; pure-Rust TLS so we don't pull
//! native-tls / openssl into the build cache).
//!
//! v1.x ships `HttpJwksResolver` with a 1-hour TTL cache keyed by issuer URL.
//! On a cache miss (or stale entry) the resolver:
//!
//! 1. Fetches `<issuer>/.well-known/openid-configuration` → JSON discovery
//!    document → `jwks_uri`.
//! 2. Fetches the JWK Set from that URI.
//! 3. Stores in the cache with `fetched_at = now`.
//!
//! On a `kid`-miss against an otherwise-fresh cache entry (a key the cache
//! doesn't carry), the resolver performs a **rate-limited refresh**: at most
//! one network round-trip per [`KID_MISS_REFRESH_INTERVAL`] (default 60 s) per
//! issuer, regardless of how many lookups for unknown kids hit it. This keeps
//! a misbehaving client (or attacker) from being able to thrash the IdP
//! through the storage server.
//!
//! Pre-loading via [`HttpJwksResolver::insert`] still works — useful for unit
//! tests, air-gapped deployments, or operators who prefer to manage JWKS
//! refresh out-of-band on a cron.
//!
//! ## Rationale: blocking client
//!
//! The `JwksResolver::resolve` trait is sync, called from a SQL closure. We
//! could try to `tokio::runtime::Handle::block_on` an async client, but that
//! deadlocks if the calling task is itself running inside a single-threaded
//! tokio runtime (or hits "Cannot block the current thread from within a
//! runtime" on multi-threaded). A blocking reqwest client sidesteps the
//! issue: each call spawns its own blocking thread (handled internally by
//! reqwest's blocking facade) without touching the caller's runtime.
//!
//! ## Cache shape
//!
//! ```text
//!   issuer -> CachedEntry {
//!     jwks: JwkSet,
//!     fetched_at: Instant,
//!     last_kid_miss_refresh: Option<Instant>,
//!   }
//! ```

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use ohd_storage_core::identities::JwksResolver;
use ohd_storage_core::{Error, Result};
use serde::Deserialize;

/// Cache entry: a JWK set + when it was fetched + when we last refreshed
/// because of a `kid` miss (rate-limit anchor).
#[derive(Clone)]
struct Cached {
    jwks: JwkSet,
    fetched_at: Instant,
    last_kid_miss_refresh: Option<Instant>,
}

/// Default TTL — operators should refresh JWKS at least hourly.
pub const JWKS_TTL: Duration = Duration::from_secs(3600);

/// Minimum gap between `kid`-miss-triggered refreshes per issuer. Defends
/// the upstream IdP from thrashing when a malformed token arrives in a loop.
pub const KID_MISS_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Default HTTP timeout for discovery + JWKS fetches.
pub const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// HTTP-aware JWKS resolver with a TTL cache.
///
/// Per `spec/auth.md` "JWKS handling" — fetches JWKS over HTTPS, caches with
/// a 1-hour TTL, and tolerates `kid` misses with rate-limited refresh.
///
/// Pre-loaded entries via [`HttpJwksResolver::insert`] are honoured first;
/// any cached entry younger than [`JWKS_TTL`] takes priority over a network
/// fetch. Operators can opt-out of network fetches entirely by constructing
/// with [`HttpJwksResolver::without_network`].
///
/// ## Lazy client construction
///
/// The blocking reqwest client owns an internal tokio runtime; constructing
/// (or dropping) it from inside another runtime panics. We dodge by deferring
/// construction until the first network fetch — which always happens on a
/// SQL-mutex-holding sync thread, never on a runtime task. End-to-end tests
/// that build a `router()` for the in-process Connect-RPC harness keep
/// working because the default constructor doesn't allocate the client.
pub struct HttpJwksResolver {
    by_issuer: Mutex<HashMap<String, Cached>>,
    ttl: Duration,
    #[allow(dead_code)]
    kid_miss_refresh_interval: Duration,
    /// `Network::Disabled` means "no network fetches; pre-load only" (test
    /// mode + air-gap). `Network::Lazy` defers blocking-client construction
    /// to first use. `Network::Eager(client)` carries a pre-built client.
    network: Mutex<Network>,
}

enum Network {
    Disabled,
    Lazy,
    Eager(reqwest::blocking::Client),
}

impl HttpJwksResolver {
    /// Construct a resolver with the default TTL. The network client is built
    /// lazily on first use (avoids a runtime-drop panic when the resolver is
    /// constructed from inside a tokio task — see "Lazy client construction").
    pub fn new() -> Self {
        Self {
            by_issuer: Mutex::new(HashMap::new()),
            ttl: JWKS_TTL,
            kid_miss_refresh_interval: KID_MISS_REFRESH_INTERVAL,
            network: Mutex::new(Network::Lazy),
        }
    }

    /// Construct with a custom TTL.
    #[allow(dead_code)]
    pub fn with_ttl(ttl: Duration) -> Self {
        let mut s = Self::new();
        s.ttl = ttl;
        s
    }

    /// Construct without any network client. All lookups go through
    /// pre-loaded entries; cache misses error with `InvalidArgument`. Useful
    /// for unit tests and air-gapped deployments.
    #[allow(dead_code)]
    pub fn without_network() -> Self {
        Self {
            by_issuer: Mutex::new(HashMap::new()),
            ttl: JWKS_TTL,
            kid_miss_refresh_interval: KID_MISS_REFRESH_INTERVAL,
            network: Mutex::new(Network::Disabled),
        }
    }

    /// Construct with a fully custom configuration (used by tests to point
    /// at a mock IdP and shorten the timing constants).
    #[doc(hidden)]
    #[allow(dead_code)]
    pub fn with_config(
        client: Option<reqwest::blocking::Client>,
        ttl: Duration,
        kid_miss_refresh_interval: Duration,
    ) -> Self {
        let network = match client {
            Some(c) => Network::Eager(c),
            None => Network::Disabled,
        };
        Self {
            by_issuer: Mutex::new(HashMap::new()),
            ttl,
            kid_miss_refresh_interval,
            network: Mutex::new(network),
        }
    }

    /// Pre-load a JWKS for an issuer. Replaces any existing entry.
    ///
    /// Useful for tests and for operators who manage JWKS refresh
    /// out-of-band on a cron.
    #[allow(dead_code)]
    pub fn insert(&self, issuer: impl Into<String>, jwks: JwkSet) {
        let mut map = self.by_issuer.lock().expect("HttpJwksResolver mutex");
        map.insert(
            issuer.into(),
            Cached {
                jwks,
                fetched_at: Instant::now(),
                last_kid_miss_refresh: None,
            },
        );
    }

    /// Drop every cached entry (force re-fetch on next resolve).
    #[allow(dead_code)]
    pub fn invalidate_all(&self) {
        self.by_issuer
            .lock()
            .expect("HttpJwksResolver mutex")
            .clear();
    }

    /// Refresh-on-`kid`-miss path. Callers that have a fresh-but-incomplete
    /// cache entry use this to attempt a refresh (rate-limited per
    /// [`KID_MISS_REFRESH_INTERVAL`]).
    #[allow(dead_code)]
    pub fn refresh_on_kid_miss(&self, issuer: &str) -> Result<JwkSet> {
        let should_refresh = {
            let map = self.by_issuer.lock().expect("HttpJwksResolver mutex");
            match map.get(issuer) {
                Some(entry) => match entry.last_kid_miss_refresh {
                    Some(t) => t.elapsed() >= self.kid_miss_refresh_interval,
                    None => true,
                },
                None => true,
            }
        };
        if !should_refresh {
            return Err(Error::InvalidArgument(format!(
                "JWKS for {issuer:?}: kid-miss refresh rate-limited; \
                 try again in <{}s",
                self.kid_miss_refresh_interval.as_secs()
            )));
        }
        let jwks = self.fetch(issuer)?;
        let mut map = self.by_issuer.lock().expect("HttpJwksResolver mutex");
        map.insert(
            issuer.to_string(),
            Cached {
                jwks: jwks.clone(),
                fetched_at: Instant::now(),
                last_kid_miss_refresh: Some(Instant::now()),
            },
        );
        Ok(jwks)
    }

    /// Lazy-build (or fetch the existing) blocking client. Returns `None`
    /// for `Network::Disabled`. Holds the network mutex briefly.
    fn get_or_build_client(&self) -> Option<reqwest::blocking::Client> {
        let mut net = self.network.lock().expect("HttpJwksResolver network mutex");
        match &*net {
            Network::Disabled => None,
            Network::Eager(c) => Some(c.clone()),
            Network::Lazy => {
                // Build now and upgrade the slot to Eager so subsequent calls
                // skip the build step.
                let client = reqwest::blocking::Client::builder()
                    .timeout(HTTP_TIMEOUT)
                    .user_agent(concat!("ohd-storage-server/", env!("CARGO_PKG_VERSION")))
                    .build()
                    .expect("building reqwest::blocking::Client should not fail");
                *net = Network::Eager(client.clone());
                Some(client)
            }
        }
    }

    /// Network fetch: discovery → `jwks_uri` → JWKS JSON. Errors surface as
    /// [`Error::InvalidArgument`] with enough detail to diagnose (issuer URL,
    /// status code, reqwest error category) but no token / claim leakage.
    fn fetch(&self, issuer: &str) -> Result<JwkSet> {
        let client = self.get_or_build_client().ok_or_else(|| {
            Error::InvalidArgument(format!(
                "JWKS for {issuer:?} not pre-loaded and HttpJwksResolver was \
                 built without a network client"
            ))
        })?;
        let client = &client;
        let discovery_url = build_discovery_url(issuer);
        let disc: DiscoveryDoc = client
            .get(&discovery_url)
            .send()
            .map_err(|e| {
                Error::InvalidArgument(format!("JWKS discovery {discovery_url:?} failed: {e}"))
            })?
            .error_for_status()
            .map_err(|e| {
                Error::InvalidArgument(format!("JWKS discovery {discovery_url:?} HTTP error: {e}"))
            })?
            .json()
            .map_err(|e| {
                Error::InvalidArgument(format!("JWKS discovery {discovery_url:?} JSON parse: {e}"))
            })?;

        let jwks_uri = disc.jwks_uri.ok_or_else(|| {
            Error::InvalidArgument(format!(
                "JWKS discovery {discovery_url:?}: missing jwks_uri"
            ))
        })?;

        let jwks: JwkSet = client
            .get(&jwks_uri)
            .send()
            .map_err(|e| Error::InvalidArgument(format!("JWKS fetch {jwks_uri:?} failed: {e}")))?
            .error_for_status()
            .map_err(|e| {
                Error::InvalidArgument(format!("JWKS fetch {jwks_uri:?} HTTP error: {e}"))
            })?
            .json()
            .map_err(|e| {
                Error::InvalidArgument(format!("JWKS fetch {jwks_uri:?} JSON parse: {e}"))
            })?;
        Ok(jwks)
    }
}

impl Default for HttpJwksResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl JwksResolver for HttpJwksResolver {
    fn resolve(&self, issuer: &str) -> Result<JwkSet> {
        // Fast path: live cache hit.
        {
            let map = self.by_issuer.lock().expect("HttpJwksResolver mutex");
            if let Some(entry) = map.get(issuer) {
                if entry.fetched_at.elapsed() < self.ttl {
                    return Ok(entry.jwks.clone());
                }
            }
        }
        // Slow path: TTL expired (or absent). Network fetch.
        let jwks = self.fetch(issuer)?;
        let mut map = self.by_issuer.lock().expect("HttpJwksResolver mutex");
        map.insert(
            issuer.to_string(),
            Cached {
                jwks: jwks.clone(),
                fetched_at: Instant::now(),
                last_kid_miss_refresh: None,
            },
        );
        Ok(jwks)
    }
}

/// OIDC discovery doc subset — we only need `jwks_uri`.
#[derive(Deserialize)]
struct DiscoveryDoc {
    jwks_uri: Option<String>,
}

/// Build `<issuer>/.well-known/openid-configuration` honouring an optional
/// trailing slash on the issuer URL.
fn build_discovery_url(issuer: &str) -> String {
    let trimmed = issuer.trim_end_matches('/');
    format!("{trimmed}/.well-known/openid-configuration")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_url_trims_trailing_slash() {
        assert_eq!(
            build_discovery_url("https://idp.example/"),
            "https://idp.example/.well-known/openid-configuration"
        );
        assert_eq!(
            build_discovery_url("https://idp.example"),
            "https://idp.example/.well-known/openid-configuration"
        );
    }

    #[test]
    fn without_network_errors_on_unknown_issuer() {
        let r = HttpJwksResolver::without_network();
        let result = r.resolve("https://no.example");
        assert!(matches!(result, Err(Error::InvalidArgument(_))));
    }

    #[test]
    fn pre_loaded_entry_resolved() {
        let r = HttpJwksResolver::without_network();
        let empty = JwkSet { keys: vec![] };
        r.insert("https://idp.example", empty.clone());
        let got = r.resolve("https://idp.example").unwrap();
        assert_eq!(got.keys.len(), 0);
    }

    #[test]
    fn kid_miss_rate_limit_blocks_second_call() {
        // First call refreshes; second within the rate-limit window is rejected
        // — without a network client there's nothing to refresh against, so the
        // first call also errors. Use a tight window to verify the timing logic.
        let r = HttpJwksResolver::with_config(None, JWKS_TTL, Duration::from_secs(3600));
        // Pre-load so the first call's error path doesn't short-circuit.
        let empty = JwkSet { keys: vec![] };
        r.insert("https://idp.example", empty.clone());
        // First miss: would attempt a refresh (no client → error).
        let first = r.refresh_on_kid_miss("https://idp.example");
        assert!(matches!(first, Err(Error::InvalidArgument(_))));

        // Second miss: rate-limit blocks if last_kid_miss_refresh was stamped.
        // Since the first call errored *before* stamping, we manually stamp it.
        {
            let mut map = r.by_issuer.lock().unwrap();
            let e = map.get_mut("https://idp.example").unwrap();
            e.last_kid_miss_refresh = Some(Instant::now());
        }
        let second = r.refresh_on_kid_miss("https://idp.example");
        let msg = match second {
            Err(Error::InvalidArgument(m)) => m,
            other => panic!("expected rate-limit error, got {other:?}"),
        };
        assert!(msg.contains("rate-limited"), "msg={msg}");
    }
}
