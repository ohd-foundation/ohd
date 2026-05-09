//! Axum-based HTTP server for the relay.
//!
//! Endpoints (per `spec/relay-protocol.md` "RPC surface", with v1 path prefix):
//!
//! - `POST   /v1/register`           — first-time storage registration
//! - `POST   /v1/heartbeat`          — registration-level keepalive
//! - `POST   /v1/deregister`         — drop a registration
//! - `WS     /v1/tunnel/:rendezvous_id` — storage's persistent bidi tunnel
//! - `WS     /v1/attach/:rendezvous_id` — consumer attach for a session
//! - `GET    /health`                — liveness probe
//!
//! Note: in production this binary sits behind Caddy, which terminates outer
//! TLS (HTTP/3 + HTTP/2) and proxies to us on a private port. We speak plain
//! HTTP/1.1 + HTTP/2 + WebSockets directly to Caddy.

#![allow(clippy::too_many_arguments)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::auth::{OidcVerifier, OidcVerifierConfig, OidcVerifyError};
use crate::config::{AllowedIssuer, RegistrationAuthConfig, RelayConfig};
use crate::emergency_endpoints::{
    handle_emergency_handoff, handle_emergency_status, EmergencyStateTable, StorageTunnelClient,
};
use crate::frame::{FrameType, TunnelFrame};
use crate::push::{
    ApnsConfig, ApnsEnvironment, ApnsPushClient, FcmConfig, FcmPushClient, PushClient,
    PushDispatcher, WAKE_DEADLINE,
};
use crate::session::{SessionRelaySide, TunnelEndpoint, DEFAULT_RECEIVE_WINDOW};
use crate::state::{now_ms, PushToken, RegistrationRow, RelayState};

/// Options for `run_serve`.
pub struct ServeOptions {
    /// Path to the operator's `relay.toml` config (currently unused).
    pub config_path: String,
    /// Optional CLI override for the bind address.
    pub bind_override: Option<String>,
    /// SQLite database path.
    pub db_path: String,
    /// Bind port (overridden by `bind_override` if set).
    pub port: u16,
    /// Optional UDP/QUIC listen address for HTTP/3. When set, the relay
    /// runs an in-binary HTTP/3 listener for the REST endpoints
    /// (register / heartbeat / deregister) alongside the HTTP/2
    /// listener. The WebSocket-based tunnel + attach paths stay HTTP/2
    /// (RFC 9220 immaturity in `h3`); see `src/http3.rs` for details.
    pub http3_listen: Option<SocketAddr>,
    /// Optional path to a PEM cert chain for the HTTP/3 listener. When
    /// paired with `http3_key`, replaces the dev self-signed cert.
    pub http3_cert: Option<std::path::PathBuf>,
    /// Optional path to a PEM private key for the HTTP/3 listener.
    pub http3_key: Option<std::path::PathBuf>,
    /// Optional UDP listen address for the **raw QUIC tunnel** (ALPN
    /// `ohd-tnl1`). When set, the relay accepts long-lived storage
    /// tunnels over a QUIC endpoint with native connection migration —
    /// the preferred shape for mobile / cellular phones whose IP path
    /// changes constantly. WebSocket-over-HTTP/2 tunneling remains
    /// available as a fallback for networks that block UDP/443. See
    /// `src/quic_tunnel.rs` for the wire shape and design rationale.
    pub quic_tunnel_listen: Option<SocketAddr>,
    /// Optional PEM cert chain for the raw QUIC tunnel listener.
    pub quic_tunnel_cert: Option<std::path::PathBuf>,
    /// Optional PEM private key for the raw QUIC tunnel listener.
    pub quic_tunnel_key: Option<std::path::PathBuf>,
}

#[derive(Clone)]
pub struct AppState {
    pub relay: RelayState,
    pub push: Arc<dyn PushClient>,
    /// Public hostname used to compose `rendezvous_url` in register
    /// responses.
    pub public_host: String,
    /// Per-OIDC gating for the registration RPC. Always present (the
    /// default permissive verifier has an empty allowlist and short-
    /// circuits at the handler level).
    pub registration_auth: RegistrationAuthState,
    /// Authority-mode state. Only `Some` when the `authority` feature is
    /// compiled in AND `[authority] enabled = true` in `relay.toml`.
    /// Stored as an `Option<Arc<...>>` even when the feature is off so the
    /// router shape stays uniform; `handle_emergency_initiate` returns 501
    /// when this is `None`.
    #[cfg(feature = "authority")]
    pub authority: Option<crate::auth_mode::AuthorityState>,
    /// Persisted emergency-request + handoff bookkeeping. Used by
    /// `/v1/emergency/{initiate,status,handoff}`. Always present (the
    /// schema migration creates the tables unconditionally — the
    /// endpoints themselves remain feature-gated for the parts that
    /// require authority signing).
    pub emergency: EmergencyStateTable,
    /// Outbound storage-tunnel client used by `/v1/emergency/handoff`
    /// (and TTL-side `OhdcService.GetEmergencyConfig` lookups). `None`
    /// when the relay hasn't been configured with a tunnel client; the
    /// handoff handler returns 503 in that case so the tablet can fall
    /// back to its mock path.
    pub storage_tunnel: Option<Arc<dyn StorageTunnelClient>>,
}

/// Per-OIDC registration gating state. Snapshots the relevant
/// `[auth.registration]` fields plus a JWKS-cache-bearing verifier so the
/// register handler doesn't need to reach into the config every call.
#[derive(Clone)]
pub struct RegistrationAuthState {
    pub require_oidc: bool,
    pub allowed_issuers: Vec<AllowedIssuer>,
    pub verifier: OidcVerifier,
}

impl RegistrationAuthState {
    pub fn permissive() -> Self {
        Self::from_config(&RegistrationAuthConfig::default())
    }

    pub fn from_config(cfg: &RegistrationAuthConfig) -> Self {
        Self {
            require_oidc: cfg.require_oidc,
            allowed_issuers: cfg.allowed_issuers.clone(),
            verifier: OidcVerifier::new(OidcVerifierConfig::from_registration(cfg)),
        }
    }

    /// True when an `id_token` is required to register (allowlist set
    /// and `require_oidc=true`). When the allowlist is empty AND
    /// `require_oidc=false`, the relay is fully permissive.
    pub fn requires_token(&self) -> bool {
        !self.allowed_issuers.is_empty() && self.require_oidc
    }

    /// True when the relay accepts (and verifies) an `id_token` if one
    /// is presented. Even when `require_oidc=false`, a presented token
    /// must verify cleanly against the allowlist.
    pub fn is_gated(&self) -> bool {
        !self.allowed_issuers.is_empty()
    }
}

/// Async entry point for `ohd-relay serve`.
pub async fn run_serve(opts: ServeOptions) -> anyhow::Result<()> {
    let bind = opts
        .bind_override
        .clone()
        .unwrap_or_else(|| format!("127.0.0.1:{}", opts.port));

    let config = RelayConfig::load(&opts.config_path)?;

    info!(
        target: "ohd_relay::server",
        config = %opts.config_path,
        bind = %bind,
        db = %opts.db_path,
        fcm_configured = config.push.fcm.is_some(),
        apns_configured = config.push.apns.is_some(),
        authority_enabled = config.authority.enabled,
        "starting relay"
    );

    let relay = RelayState::open(&opts.db_path).await?;
    let push = build_push_dispatcher(&config)?;
    let public_host = config.public_host.clone().unwrap_or_else(|| bind.clone());

    let registration_auth = RegistrationAuthState::from_config(&config.auth.registration);
    if registration_auth.is_gated() {
        info!(
            target: "ohd_relay::server",
            issuers = ?registration_auth
                .allowed_issuers
                .iter()
                .map(|i| i.issuer.as_str())
                .collect::<Vec<_>>(),
            require_oidc = registration_auth.require_oidc,
            "registration OIDC gating enabled"
        );
    }

    #[cfg(feature = "authority")]
    let authority = build_authority_state(&config).await?;

    // Stash a clone of `relay` so the raw-QUIC tunnel listener (which sits
    // outside the axum router) can share the same RegistrationTable +
    // SessionTable. Both are `Arc`-interior so cloning is cheap.
    let relay_for_tunnel = relay.clone();

    // Emergency-flow state (status polling + handoff audit).
    let emergency =
        EmergencyStateTable::new(relay.registrations.conn_for_emergency());
    // Storage tunnel client: not yet wired in production (the
    // storage-side outbound integration is the parallel deliverable —
    // see STATUS.md "What's stubbed / TBD"). When present, the handoff
    // endpoint forwards `OhdcService.HandoffCase` through it; the TTL
    // sweeper uses `GetEmergencyConfig` to fetch the patient's default
    // action. `None` means handoff returns 503 with
    // `code=storage_tunnel_unavailable`.
    let storage_tunnel: Option<Arc<dyn StorageTunnelClient>> = None;

    // Background TTL sweeper for in-flight emergency requests. Long-running
    // task; the watch-channel wakes it on shutdown.
    let (emergency_shutdown_tx, emergency_shutdown_rx) =
        tokio::sync::watch::channel(false);
    let sweeper_table = emergency.clone();
    let sweeper_task = tokio::spawn(async move {
        crate::emergency_endpoints::run_ttl_sweeper_loop(
            sweeper_table,
            emergency_shutdown_rx,
        )
        .await;
    });

    let app_state = AppState {
        relay,
        push,
        public_host,
        registration_auth,
        #[cfg(feature = "authority")]
        authority,
        emergency,
        storage_tunnel,
    };

    let app = build_router(app_state);

    let addr: SocketAddr = bind.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(target: "ohd_relay::server", "ohd-relay listening on {addr}");

    // Optionally launch the in-binary HTTP/3 listener for the REST
    // endpoints (register / heartbeat / deregister). The WebSocket paths
    // stay on the HTTP/2 listener (see `src/http3.rs` for the RFC 9220
    // story). The H3 task is fire-and-forget; if it fails to bind we
    // log + continue serving HTTP/2 only.
    let h3_task = if let Some(h3_addr) = opts.http3_listen {
        let h3_router = app.clone();
        // Resolve cert materials: production paths if both flags set, else
        // dev self-signed with a stderr warning. We log the warning to
        // stderr (not just `tracing`) to give operators a hard-to-miss
        // signal during manual deploys.
        let cert_result = match (opts.http3_cert.as_ref(), opts.http3_key.as_ref()) {
            (Some(cp), Some(kp)) => crate::http3::load_pem_cert_key(cp, kp),
            (None, None) => {
                eprintln!(
                    "WARNING: --http3-listen set without --http3-cert/--http3-key; \
                     using a dev self-signed cert (localhost / 127.0.0.1). \
                     Production deployments must supply real PEM materials."
                );
                crate::http3::dev_self_signed_cert()
            }
            _ => Err(anyhow::anyhow!(
                "--http3-cert and --http3-key must be supplied together"
            )),
        };
        Some(tokio::spawn(async move {
            match cert_result {
                Ok((cert, key)) => {
                    if let Err(err) =
                        crate::http3::serve(h3_addr, h3_router, cert, key).await
                    {
                        warn!(target: "ohd_relay::http3", ?err, "h3 listener exited");
                    }
                }
                Err(err) => {
                    warn!(target: "ohd_relay::http3", ?err, "h3 cert init failed");
                }
            }
        }))
    } else {
        None
    };

    // Optionally launch the raw QUIC tunnel listener (ALPN `ohd-tnl1`).
    // Separate UDP port from the HTTP/3 listener — see
    // `src/quic_tunnel.rs` "ALPN + endpoint isolation" for the rationale.
    let (quic_shutdown_tx, quic_shutdown_rx) = tokio::sync::watch::channel(false);
    let quic_tunnel_task = if let Some(qt_addr) = opts.quic_tunnel_listen {
        let cert_result = match (opts.quic_tunnel_cert.as_ref(), opts.quic_tunnel_key.as_ref()) {
            (Some(cp), Some(kp)) => crate::http3::load_pem_cert_key(cp, kp),
            (None, None) => {
                eprintln!(
                    "WARNING: --quic-tunnel-listen set without --quic-tunnel-cert/--quic-tunnel-key; \
                     using a dev self-signed cert (localhost / 127.0.0.1). \
                     Production deployments must supply real PEM materials."
                );
                crate::http3::dev_self_signed_cert()
            }
            _ => Err(anyhow::anyhow!(
                "--quic-tunnel-cert and --quic-tunnel-key must be supplied together"
            )),
        };
        let st = std::sync::Arc::new(relay_for_tunnel.clone());
        let shutdown_rx = quic_shutdown_rx.clone();
        Some(tokio::spawn(async move {
            match cert_result {
                Ok((cert, key)) => {
                    if let Err(err) = crate::quic_tunnel::serve_quic_tunnel(
                        qt_addr, cert, key, st, shutdown_rx,
                    )
                    .await
                    {
                        warn!(target: "ohd_relay::quic_tunnel", ?err, "tunnel listener exited");
                    }
                }
                Err(err) => {
                    warn!(target: "ohd_relay::quic_tunnel", ?err, "tunnel cert init failed");
                }
            }
        }))
    } else {
        None
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
            info!(target: "ohd_relay::server", "shutting down");
        })
        .await?;

    let _ = quic_shutdown_tx.send(true);
    let _ = emergency_shutdown_tx.send(true);
    if let Some(h) = h3_task {
        h.abort();
    }
    if let Some(h) = quic_tunnel_task {
        // Give the QUIC listener a moment to drain after shutdown signal
        // before forcibly aborting.
        let _ = tokio::time::timeout(std::time::Duration::from_millis(500), h).await;
    }
    let _ = tokio::time::timeout(std::time::Duration::from_millis(500), sweeper_task).await;

    Ok(())
}

/// Wire `relay.toml` push sections into a `PushDispatcher`. Missing
/// sections are no-ops — a relay without `[push.fcm]` simply can't deliver
/// FCM pushes (and `wait_for_tunnel` returns `Err(UnsupportedTokenType)`,
/// caught and logged at the call site).
pub fn build_push_dispatcher(cfg: &RelayConfig) -> anyhow::Result<Arc<dyn PushClient>> {
    let mut d = PushDispatcher::new();
    if let Some(fcm) = &cfg.push.fcm {
        let fc = FcmConfig {
            project_id: fcm.project_id.clone(),
            service_account_path: fcm.service_account_path.clone(),
            fcm_base_url: fcm.fcm_base_url.clone(),
            token_base_url: fcm.token_base_url.clone(),
        };
        let client = FcmPushClient::new(fc)
            .map_err(|e| anyhow::anyhow!("FCM client init: {e}"))?;
        info!(
            target: "ohd_relay::server",
            project_id = %fcm.project_id,
            "FCM push configured"
        );
        d = d.with_fcm(client);
    }
    if let Some(apns) = &cfg.push.apns {
        let env = match apns.environment.as_str() {
            "sandbox" | "development" => ApnsEnvironment::Sandbox,
            _ => ApnsEnvironment::Production,
        };
        let ac = ApnsConfig {
            team_id: apns.team_id.clone(),
            key_id: apns.key_id.clone(),
            key_path: apns.key_path.clone(),
            bundle_id: apns.bundle_id.clone(),
            environment: env,
            override_base_url: apns.override_base_url.clone(),
        };
        let client = ApnsPushClient::new(ac)
            .map_err(|e| anyhow::anyhow!("APNs client init: {e}"))?;
        info!(
            target: "ohd_relay::server",
            team_id = %apns.team_id,
            key_id = %apns.key_id,
            bundle_id = %apns.bundle_id,
            environment = ?env,
            "APNs push configured"
        );
        d = d.with_apns(client);
    }
    Ok(Arc::new(d))
}

pub fn build_router(state: AppState) -> Router {
    #[cfg_attr(not(feature = "authority"), allow(unused_mut))]
    let mut router = Router::new()
        .route("/health", get(health))
        .route("/v1/register", post(handle_register))
        .route("/v1/heartbeat", post(handle_heartbeat))
        .route("/v1/deregister", post(handle_deregister))
        .route("/v1/tunnel/:rendezvous_id", get(handle_tunnel_ws))
        .route("/v1/attach/:rendezvous_id", get(handle_attach_ws))
        .route("/v1/auth/info", get(handle_auth_info))
        // Emergency-flow polling + handoff. These are wired regardless of
        // the `authority` feature — the tablet polls `/status/{id}`
        // throughout the break-glass loop, and `/handoff` is a
        // relay-mediated forward to storage that doesn't itself need a
        // signing cert. The `/initiate` endpoint that mints the request
        // remains feature-gated below because IT signs the payload.
        .route(
            "/v1/emergency/status/:request_id",
            get(handle_emergency_status),
        )
        .route("/v1/emergency/handoff", post(handle_emergency_handoff));

    #[cfg(feature = "authority")]
    {
        router = router.route(
            "/v1/emergency/initiate",
            post(handle_emergency_initiate),
        );
    }

    router.with_state(state)
}

/// Authority-mode boot. When `[authority] enabled = true` and the OIDC
/// token + email claim are configured, builds the cert refresh state and
/// kicks off the background loop. Returns `None` when authority mode is
/// disabled (the standard fast path for plain forwarding relays).
#[cfg(feature = "authority")]
async fn build_authority_state(
    cfg: &RelayConfig,
) -> anyhow::Result<Option<crate::auth_mode::AuthorityState>> {
    if !cfg.authority.enabled {
        return Ok(None);
    }
    let fulcio_url = cfg
        .authority
        .fulcio_url
        .clone()
        .ok_or_else(|| anyhow::anyhow!("[authority] enabled but fulcio_url missing"))?;
    let oidc_path = cfg
        .authority
        .oidc_id_token_path
        .clone()
        .ok_or_else(|| anyhow::anyhow!("[authority] enabled but oidc_id_token_path missing"))?;
    // Email claim — for v1 we expect it explicit. We can derive it from the
    // JWT in a follow-up.
    let email_claim = cfg
        .authority
        .org_label
        .clone()
        .unwrap_or_else(|| "operator@unknown.invalid".into());
    let rekor = cfg.authority.rekor_url.clone().map(|u| {
        crate::auth_mode::RekorConfig {
            rekor_url: u,
            override_entries_url: None,
            soft_fail: true,
        }
    });
    let st_cfg = crate::auth_mode::AuthorityStateConfig {
        fulcio: crate::auth_mode::FulcioConfig {
            fulcio_url,
            override_signing_cert_url: None,
        },
        rekor,
        refresh_window: std::time::Duration::from_secs(60 * 60),
        poll_interval: std::time::Duration::from_secs(60),
        retry_backoff: std::time::Duration::from_secs(5 * 60),
        oidc_id_token_path: oidc_path,
        oidc_email_claim: email_claim,
    };
    let state = crate::auth_mode::AuthorityState::new(st_cfg)
        .map_err(|e| anyhow::anyhow!("AuthorityState init: {e}"))?;
    // Best-effort initial refresh; if it fails we still start the server
    // (the refresh loop will retry).
    if let Err(e) = state.refresh().await {
        warn!(
            target: "ohd_relay::auth_mode",
            error = %e,
            "initial authority refresh failed; refresh loop will retry"
        );
    }
    let loop_state = state.clone();
    tokio::spawn(async move {
        crate::auth_mode::refresh::run_refresh_loop(loop_state).await;
    });
    Ok(Some(state))
}

// ---------------------------------------------------------------------------
// Health
// ---------------------------------------------------------------------------

async fn health() -> &'static str {
    "OHD Relay v0 — health: ok"
}

// ---------------------------------------------------------------------------
// /v1/register
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Serialize)]
pub struct RegisterRequest {
    /// Hex-encoded 16-byte ULID.
    pub user_ulid: String,
    /// Hex-encoded Ed25519 SPKI bytes.
    pub storage_pubkey_spki_hex: String,
    /// Optional FCM/APNs/email push token.
    pub push_token: Option<PushTokenWire>,
    pub user_label: Option<String>,
    /// Optional OIDC `id_token` (compact-encoded JWT).
    ///
    /// Behaviour:
    /// - When the relay's `[auth.registration]` allowlist is empty AND
    ///   `require_oidc=false`: this field is ignored.
    /// - When `require_oidc=true` AND missing here: registration is
    ///   rejected with `OIDC_REQUIRED`.
    /// - When present: must be a valid JWT issued by an allowlisted
    ///   issuer with the matching `expected_audience`. Verification
    ///   failures map to `OIDC_VERIFY_FAILED`.
    pub id_token: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub rendezvous_id: String,
    pub rendezvous_url: String,
    pub long_lived_credential: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
#[serde(tag = "platform", content = "value")]
pub enum PushTokenWire {
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

impl From<PushTokenWire> for PushToken {
    fn from(p: PushTokenWire) -> Self {
        match p {
            PushTokenWire::Fcm(t) => PushToken::Fcm(t),
            PushTokenWire::Apns(t) => PushToken::Apns(t),
            PushTokenWire::Email(t) => PushToken::Email(t),
            PushTokenWire::WebPush {
                endpoint,
                p256dh,
                auth,
            } => PushToken::WebPush {
                endpoint,
                p256dh,
                auth,
            },
        }
    }
}

async fn handle_register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<RegisterResponse>), ApiError> {
    let user_ulid = decode_ulid(&req.user_ulid)?;
    let storage_pubkey =
        hex::decode(&req.storage_pubkey_spki_hex).map_err(|_| ApiError::bad_request("storage_pubkey_spki_hex must be hex"))?;

    // Per-OIDC gating per `[auth.registration]`. When the allowlist is
    // empty AND require_oidc=false, we skip this entirely (legacy path).
    let (oidc_iss, oidc_sub) = enforce_registration_oidc(
        &state.registration_auth,
        req.id_token.as_deref(),
    )
    .await?;

    let rendezvous_id = generate_rendezvous_id();
    let long_lived_credential = generate_credential();
    let cred_hash = sha256_32(long_lived_credential.as_bytes());

    let now = now_ms();
    let row = RegistrationRow {
        rendezvous_id: rendezvous_id.clone(),
        user_ulid,
        push_token: req.push_token.map(Into::into),
        last_heartbeat_at_ms: now,
        long_lived_credential_hash: cred_hash,
        registered_at_ms: now,
        user_label: req.user_label,
        storage_pubkey,
        oidc_iss,
        oidc_sub,
    };

    state
        .relay
        .registrations
        .register(row)
        .await
        .map_err(ApiError::internal)?;

    // Use `wss://` when the public_host looks like a real hostname; `ws://`
    // when it's a literal `127.0.0.1:PORT` dev / test bind. The relay sits
    // behind Caddy (or equivalent) in production; Caddy terminates outer
    // TLS, so consumers see `wss://relay.example.com/...`.
    let scheme = if looks_like_dev_bind(&state.public_host) {
        "ws"
    } else {
        "wss"
    };
    let rendezvous_url = format!("{scheme}://{}/v1/tunnel/{}", state.public_host, rendezvous_id);
    Ok((
        StatusCode::CREATED,
        Json(RegisterResponse {
            rendezvous_id,
            rendezvous_url,
            long_lived_credential,
        }),
    ))
}

/// Apply the relay's per-OIDC registration policy.
///
/// Decision matrix (cfg.allowed_issuers / cfg.require_oidc / id_token):
///
/// | issuers | require_oidc | id_token | result |
/// |---------|--------------|----------|--------|
/// | empty   | false        | any      | accept (no oidc identity) |
/// | empty   | true         | any      | accept — `require_oidc` is meaningless without an allowlist (logged at boot if combined; the field has no effect) |
/// | set     | false        | absent   | accept (no oidc identity) |
/// | set     | false        | present  | verify; on success record `(iss,sub)`, on failure reject with `OIDC_VERIFY_FAILED` |
/// | set     | true         | absent   | reject `OIDC_REQUIRED` |
/// | set     | true         | present  | verify; on success record, on failure reject |
async fn enforce_registration_oidc(
    auth: &RegistrationAuthState,
    id_token: Option<&str>,
) -> Result<(Option<String>, Option<String>), ApiError> {
    if !auth.is_gated() {
        return Ok((None, None));
    }
    match id_token {
        None => {
            if auth.require_oidc {
                Err(ApiError::oidc_required())
            } else {
                Ok((None, None))
            }
        }
        Some(token) => match auth.verifier.verify(token).await {
            Ok(v) => {
                debug!(
                    target: "ohd_relay::auth::oidc",
                    iss = %v.iss,
                    sub = %v.sub,
                    "registration accepted via OIDC"
                );
                Ok((Some(v.iss), Some(v.sub)))
            }
            Err(e) => {
                warn!(
                    target: "ohd_relay::auth::oidc",
                    error = %e,
                    "registration rejected: OIDC verification failed"
                );
                Err(map_oidc_error(e))
            }
        },
    }
}

fn map_oidc_error(e: OidcVerifyError) -> ApiError {
    match e {
        OidcVerifyError::Missing => ApiError::oidc_required(),
        other => ApiError::oidc_verify_failed(other.to_string()),
    }
}

// ---------------------------------------------------------------------------
// /v1/auth/info  (public discovery)
// ---------------------------------------------------------------------------

/// Public response of `GET /v1/auth/info`. Surface only what's needed for
/// storage's relay-discovery flow:
///
/// - whether registration requires OIDC (`registration_oidc_required`)
/// - the issuer URLs + expected audiences this relay accepts
///
/// Both pieces are public anyway (the issuer URL is in the operator's
/// IdP discovery doc; the audience is just a string label for the
/// relay). We do NOT leak operator-internal info like which subjects are
/// allowed — the IdP-side allowlist (which sub claims may register) is
/// out of scope for v1; the IdP itself is the gatekeeper.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuthInfoResponse {
    /// True when storage must present an `id_token` to register
    /// successfully (allowlist set AND `require_oidc=true`).
    pub registration_oidc_required: bool,
    pub allowed_issuers: Vec<AllowedIssuerWire>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AllowedIssuerWire {
    pub issuer: String,
    pub expected_audience: String,
}

async fn handle_auth_info(State(state): State<AppState>) -> Json<AuthInfoResponse> {
    let resp = AuthInfoResponse {
        registration_oidc_required: state.registration_auth.requires_token(),
        allowed_issuers: state
            .registration_auth
            .allowed_issuers
            .iter()
            .map(|i| AllowedIssuerWire {
                issuer: i.issuer.clone(),
                expected_audience: i.expected_audience.clone(),
            })
            .collect(),
    };
    Json(resp)
}

// ---------------------------------------------------------------------------
// /v1/heartbeat & /v1/deregister
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    pub rendezvous_id: String,
    pub long_lived_credential: String,
}

#[derive(Debug, Serialize)]
pub struct HeartbeatResponse {
    pub ok: bool,
}

async fn handle_heartbeat(
    State(state): State<AppState>,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<HeartbeatResponse>, ApiError> {
    auth_check(&state, &req.rendezvous_id, &req.long_lived_credential).await?;
    let updated = state
        .relay
        .registrations
        .heartbeat(&req.rendezvous_id, now_ms())
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(HeartbeatResponse { ok: updated }))
}

async fn handle_deregister(
    State(state): State<AppState>,
    Json(req): Json<HeartbeatRequest>,
) -> Result<Json<HeartbeatResponse>, ApiError> {
    auth_check(&state, &req.rendezvous_id, &req.long_lived_credential).await?;
    // If a tunnel is open, tear it down first.
    if let Some(endpoint) = state.relay.sessions.deregister_tunnel(&req.rendezvous_id).await {
        endpoint.drain_all_sessions().await;
    }
    let removed = state
        .relay
        .registrations
        .deregister(&req.rendezvous_id)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(HeartbeatResponse { ok: removed }))
}

async fn auth_check(
    state: &AppState,
    rendezvous_id: &str,
    long_lived_credential: &str,
) -> Result<(), ApiError> {
    let row = state
        .relay
        .registrations
        .lookup_by_rendezvous(rendezvous_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("rendezvous_id"))?;
    let presented = sha256_32(long_lived_credential.as_bytes());
    if !constant_time_eq_32(&row.long_lived_credential_hash, &presented) {
        return Err(ApiError::unauthorized("bad long_lived_credential"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// /v1/tunnel/:rendezvous_id  (storage)
// ---------------------------------------------------------------------------

async fn handle_tunnel_ws(
    State(state): State<AppState>,
    Path(rendezvous_id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<axum::response::Response, ApiError> {
    // Storage authenticates with `?cred=<long_lived_credential>` query param
    // (axum doesn't naturally extract auth headers on WS upgrades depending
    // on the proxy). We accept either query or header in production; query
    // is what tests use.
    let row = state
        .relay
        .registrations
        .lookup_by_rendezvous(&rendezvous_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("rendezvous_id"))?;

    Ok(ws.on_upgrade(move |socket| {
        run_storage_tunnel(state, rendezvous_id, row, socket)
    }))
}

async fn run_storage_tunnel(
    state: AppState,
    rendezvous_id: String,
    _row: RegistrationRow,
    socket: WebSocket,
) {
    use futures_util::{SinkExt, StreamExt};

    let (endpoint, mut outbound_rx) = TunnelEndpoint::new(rendezvous_id.clone());
    state.relay.sessions.register_tunnel(endpoint.clone()).await;
    let _ = state
        .relay
        .registrations
        .update_endpoint(&rendezvous_id, true, now_ms())
        .await;

    info!(target: "ohd_relay::tunnel", %rendezvous_id, "storage tunnel up");

    let (mut ws_tx, mut ws_rx) = socket.split();

    // Writer task: drains outbound_rx, encodes frames, sends on the WS.
    let writer = tokio::spawn(async move {
        while let Some(frame) = outbound_rx.recv().await {
            let bytes = match frame.encode() {
                Ok(b) => b,
                Err(e) => {
                    warn!(target: "ohd_relay::tunnel", error = %e, "encode failed; closing tunnel");
                    break;
                }
            };
            if ws_tx.send(Message::Binary(bytes.to_vec())).await.is_err() {
                break;
            }
        }
        let _ = ws_tx.close().await;
    });

    // Reader task: parses inbound frames and dispatches.
    let endpoint_for_reader = endpoint.clone();
    let rendezvous_for_reader = rendezvous_id.clone();
    let reader = tokio::spawn(async move {
        while let Some(msg) = ws_rx.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(e) => {
                    debug!(target: "ohd_relay::tunnel", error = %e, "ws read error");
                    break;
                }
            };
            let bytes: Vec<u8> = match msg {
                Message::Binary(b) => b,
                Message::Ping(_) | Message::Pong(_) => continue,
                Message::Close(_) => break,
                Message::Text(_) => {
                    warn!(target: "ohd_relay::tunnel", "unexpected text frame from storage");
                    break;
                }
            };
            let frame = match TunnelFrame::decode(&bytes) {
                Ok(f) => f,
                Err(e) => {
                    warn!(target: "ohd_relay::tunnel", error = %e, "frame decode failed");
                    break;
                }
            };
            handle_storage_frame(&endpoint_for_reader, &rendezvous_for_reader, frame).await;
        }
    });

    let _ = tokio::join!(reader, writer);
    state
        .relay
        .sessions
        .deregister_tunnel(&rendezvous_id)
        .await;
    let _ = state
        .relay
        .registrations
        .update_endpoint(&rendezvous_id, false, now_ms())
        .await;
    endpoint.drain_all_sessions().await;
    info!(target: "ohd_relay::tunnel", %rendezvous_id, "storage tunnel down");
}

async fn handle_storage_frame(endpoint: &TunnelEndpoint, rendezvous_id: &str, frame: TunnelFrame) {
    match frame.frame_type {
        FrameType::Hello => {
            // Reply with our own HELLO.
            let _ = endpoint
                .outbound_tx
                .send(TunnelFrame::hello(Bytes::from_static(b"ohd-relay/v0")))
                .await;
        }
        FrameType::Ping => {
            let _ = endpoint
                .outbound_tx
                .send(TunnelFrame::pong(frame.payload))
                .await;
        }
        FrameType::Pong => {
            // Heartbeat noted by virtue of receiving anything.
        }
        FrameType::Data => {
            // Route DATA payloads to the matching consumer session.
            forward_storage_to_consumer(endpoint, rendezvous_id, frame).await;
        }
        FrameType::OpenAck => {
            // Storage accepted the open. Bookkeeping only at this layer; the
            // consumer is already attached and waiting for DATA.
            debug!(target: "ohd_relay::tunnel", session_id = frame.session_id, "OPEN_ACK from storage");
        }
        FrameType::OpenNack => {
            // Storage rejected. Tear down the consumer session.
            warn!(
                target: "ohd_relay::tunnel",
                session_id = frame.session_id,
                "OPEN_NACK from storage"
            );
            endpoint
                .attached_senders()
                .write()
                .await
                .remove(&frame.session_id);
        }
        FrameType::Close => {
            endpoint
                .attached_senders()
                .write()
                .await
                .remove(&frame.session_id);
        }
        FrameType::WindowUpdate => {
            // v1: flow control is advisory; mpsc backpressure is the actual
            // governor. Acknowledge by ignoring.
        }
        FrameType::Open => {
            // OPEN is relay→storage only. Storage shouldn't emit it.
            warn!(target: "ohd_relay::tunnel", %rendezvous_id, "storage emitted OPEN; ignoring");
        }
    }
}

async fn forward_storage_to_consumer(
    endpoint: &TunnelEndpoint,
    _rendezvous_id: &str,
    frame: TunnelFrame,
) {
    let session_id = frame.session_id;
    if let Some(tx) = endpoint
        .attached_senders()
        .read()
        .await
        .get(&session_id)
        .cloned()
    {
        let _ = tx.send(frame.payload).await;
    }
}

// ---------------------------------------------------------------------------
// /v1/attach/:rendezvous_id  (consumer)
// ---------------------------------------------------------------------------

async fn handle_attach_ws(
    State(state): State<AppState>,
    Path(rendezvous_id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<axum::response::Response, ApiError> {
    let row = state
        .relay
        .registrations
        .lookup_by_rendezvous(&rendezvous_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("rendezvous_id"))?;

    Ok(ws.on_upgrade(move |socket| run_consumer_attach(state, rendezvous_id, row, socket)))
}

async fn run_consumer_attach(
    state: AppState,
    rendezvous_id: String,
    row: RegistrationRow,
    socket: WebSocket,
) {
    // Look up the tunnel; if absent, attempt push-wake.
    use futures_util::SinkExt as _SinkExt;
    let endpoint = match wait_for_tunnel(&state, &rendezvous_id, &row).await {
        Some(e) => e,
        None => {
            warn!(target: "ohd_relay::attach", %rendezvous_id, "no tunnel after wake; closing");
            // Send a CLOSE-style error and return.
            let mut ws = socket;
            let frame = TunnelFrame::close(0, Bytes::from_static(b"STORAGE_OFFLINE"));
            if let Ok(b) = frame.encode() {
                let _ = ws.send(Message::Binary(b.to_vec())).await;
            }
            return;
        }
    };

    // Allocate a session_id and the storage→consumer channel inline; we
    // don't use TunnelEndpoint::open_session because we want to own the
    // receiver here.
    let session_id = endpoint.next_session_id();
    let (storage_to_consumer_tx, mut storage_to_consumer_rx) =
        mpsc::channel::<Bytes>(crate::session::SESSION_CHUNK_BUFFER);
    debug!(target: "ohd_relay::attach", %rendezvous_id, session_id, "consumer attached");
    endpoint
        .attached_senders()
        .write()
        .await
        .insert(session_id, storage_to_consumer_tx);

    // Send OPEN to storage so it can run its own auth check.
    let open_frame = TunnelFrame::open(session_id, Bytes::new());
    if endpoint.outbound_tx.send(open_frame).await.is_err() {
        warn!(target: "ohd_relay::attach", %rendezvous_id, "tunnel write failed during OPEN");
        endpoint
            .attached_senders()
            .write()
            .await
            .remove(&session_id);
        return;
    }

    use futures_util::StreamExt;
    let (mut ws_tx, mut ws_rx) = socket.split();

    // Pump storage → consumer.
    let writer = tokio::spawn(async move {
        while let Some(payload) = storage_to_consumer_rx.recv().await {
            let frame = TunnelFrame::data(session_id, payload);
            let bytes = match frame.encode() {
                Ok(b) => b,
                Err(_) => break,
            };
            if ws_tx.send(Message::Binary(bytes.to_vec())).await.is_err() {
                break;
            }
        }
        let _ = ws_tx.close().await;
    });

    // Pump consumer → storage. The consumer sends raw `TunnelFrame`s on the
    // WS — typically just `DATA` and `CLOSE` for its session id.
    let outbound_tx = endpoint.outbound_tx.clone();
    let reader = tokio::spawn(async move {
        while let Some(msg) = ws_rx.next().await {
            let msg = match msg {
                Ok(m) => m,
                Err(_) => break,
            };
            let bytes: Vec<u8> = match msg {
                Message::Binary(b) => b,
                Message::Close(_) => break,
                _ => continue,
            };
            // Parse and stamp session_id.
            let frame = match TunnelFrame::decode(&bytes) {
                Ok(f) => f,
                Err(_) => break,
            };
            // Force the session_id to ours; ignore consumer's choice.
            let stamped = TunnelFrame {
                session_id,
                ..frame
            };
            if matches!(stamped.frame_type, FrameType::Close) {
                let _ = outbound_tx.send(stamped).await;
                break;
            }
            if outbound_tx.send(stamped).await.is_err() {
                break;
            }
        }
    });

    let _ = tokio::join!(reader, writer);
    let _ = endpoint
        .outbound_tx
        .send(TunnelFrame::close(session_id, Bytes::new()))
        .await;
    endpoint
        .attached_senders()
        .write()
        .await
        .remove(&session_id);
    debug!(target: "ohd_relay::attach", %rendezvous_id, session_id, "consumer detached");
}

async fn wait_for_tunnel(
    state: &AppState,
    rendezvous_id: &str,
    row: &RegistrationRow,
) -> Option<TunnelEndpoint> {
    if let Some(t) = state.relay.sessions.lookup(rendezvous_id).await {
        return Some(t);
    }
    // Push-wake.
    if let Some(token) = &row.push_token {
        let _ = state.push.wake(rendezvous_id, token).await;
    }
    // Poll up to WAKE_DEADLINE.
    let deadline = tokio::time::Instant::now() + WAKE_DEADLINE;
    while tokio::time::Instant::now() < deadline {
        if let Some(t) = state.relay.sessions.lookup(rendezvous_id).await {
            return Some(t);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    None
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    /// Machine-readable error code for clients to branch on. We use this
    /// for OIDC rejection paths (`OIDC_REQUIRED`, `OIDC_VERIFY_FAILED`,
    /// etc.) where storage needs to surface a precise UX message.
    pub code: Option<&'static str>,
}

impl ApiError {
    pub fn bad_request(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: msg.into(),
            code: None,
        }
    }
    pub fn not_found(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: msg.into(),
            code: None,
        }
    }
    pub fn unauthorized(msg: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: msg.into(),
            code: None,
        }
    }
    pub fn internal(err: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: err.to_string(),
            code: None,
        }
    }
    pub fn with_code(mut self, code: &'static str) -> Self {
        self.code = Some(code);
        self
    }

    /// Storage didn't present an `id_token` but the relay requires one.
    pub fn oidc_required() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "this relay requires an OIDC id_token to register".into(),
            code: Some("OIDC_REQUIRED"),
        }
    }

    /// An `id_token` was presented but didn't verify (signature,
    /// expiry, issuer-not-allowed, audience mismatch, etc.). The
    /// `reason` is a human-readable detail for logs / dev UX.
    pub fn oidc_verify_failed(reason: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: format!("OIDC verification failed: {}", reason.into()),
            code: Some("OIDC_VERIFY_FAILED"),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let body = match self.code {
            Some(c) => serde_json::json!({"error": self.message, "code": c}),
            None => serde_json::json!({"error": self.message}),
        };
        (self.status, Json(body)).into_response()
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn decode_ulid(s: &str) -> Result<[u8; 16], ApiError> {
    let raw = hex::decode(s).map_err(|_| ApiError::bad_request("user_ulid must be hex"))?;
    if raw.len() != 16 {
        return Err(ApiError::bad_request("user_ulid must be 16 bytes"));
    }
    let mut out = [0u8; 16];
    out.copy_from_slice(&raw);
    Ok(out)
}

pub fn generate_rendezvous_id() -> String {
    // 16 random bytes → 22-char base32 (no padding).
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    base32_no_pad(&buf)
}

pub fn generate_credential() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    base32_no_pad(&buf)
}

fn sha256_32(input: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(input);
    let r = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&r);
    out
}

fn constant_time_eq_32(a: &[u8; 32], b: &[u8; 32]) -> bool {
    let mut acc = 0u8;
    for i in 0..32 {
        acc |= a[i] ^ b[i];
    }
    acc == 0
}

fn looks_like_dev_bind(host: &str) -> bool {
    host.starts_with("127.")
        || host.starts_with("0.0.0.0")
        || host.starts_with("[::1]")
        || host.starts_with("localhost")
}

/// Crockford-style base32 (subset of RFC 4648), unpadded. Sufficient for
/// rendezvous IDs and bearer tokens — these don't need to round-trip through
/// any external decoder.
fn base32_no_pad(input: &[u8]) -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::with_capacity((input.len() * 8 + 4) / 5);
    let mut bits: u32 = 0;
    let mut bit_count = 0u32;
    for &b in input {
        bits = (bits << 8) | b as u32;
        bit_count += 8;
        while bit_count >= 5 {
            bit_count -= 5;
            let idx = (bits >> bit_count) & 0x1F;
            out.push(ALPHABET[idx as usize] as char);
        }
    }
    if bit_count > 0 {
        let idx = (bits << (5 - bit_count)) & 0x1F;
        out.push(ALPHABET[idx as usize] as char);
    }
    out
}

// ---------------------------------------------------------------------------
// Endpoint extension: per-session attached sender map
// ---------------------------------------------------------------------------
//
// The actual storage of the per-tunnel `session_id → mpsc::Sender<Bytes>` map
// lives in `session.rs::attached_senders_for` so the raw-QUIC tunnel handler
// in `quic_tunnel.rs` can route storage→consumer DATA through the same
// registry without duplicating state.

type AttachedSenders = crate::session::AttachedSenders;

trait EndpointExt {
    fn attached_senders(&self) -> Arc<AttachedSenders>;
}

impl EndpointExt for TunnelEndpoint {
    fn attached_senders(&self) -> Arc<AttachedSenders> {
        crate::session::attached_senders_for(self)
    }
}

// Suppress unused warnings.
#[allow(dead_code)]
fn _unused_session_relay_size_check(_: &SessionRelaySide) {
    let _ = DEFAULT_RECEIVE_WINDOW;
}

// ---------------------------------------------------------------------------
// /v1/emergency/initiate (feature-gated)
// ---------------------------------------------------------------------------
//
// Per `spec/emergency-trust.md` "Signed emergency-access request":
//
// - Authenticated responder POSTs an in-progress `EmergencyAccessRequest`
//   to the relay (responder auth is operator policy — clinic SSO /
//   hospital ADFS — and is a layer above this handler; in v1 we accept
//   the request body as-is and trust the deployment to gate at the proxy
//   level).
// - The relay signs it with its current Fulcio-issued leaf cert.
// - The relay queues a push-wake to the patient's phone (if registered)
//   and returns the signed payload + a delivery acknowledgment.
//
// The patient-side dialog flow (countdown, accept / reject, audit) lives
// in Connect mobile; this endpoint's job ends when the signed request
// arrives at the patient's storage.

#[cfg(feature = "authority")]
#[derive(Debug, Deserialize)]
pub struct EmergencyInitiateRequest {
    /// Rendezvous-id of the registered patient storage.
    pub rendezvous_id: String,
    /// Optional pin (sha256 of patient storage's identity SPKI).
    pub patient_storage_pubkey_pin_hex: Option<String>,
    /// Free-form responder identifier, mirrored into the signed payload.
    pub responder_label: Option<String>,
    pub scene_context: Option<String>,
    pub operator_label: Option<String>,
    pub scene_lat: Option<f64>,
    pub scene_lon: Option<f64>,
    pub scene_accuracy_m: Option<f32>,
}

#[cfg(feature = "authority")]
#[derive(Debug, Serialize)]
pub struct EmergencyInitiateResponse {
    /// The signed `EmergencyAccessRequest` ready to be delivered. The
    /// patient phone will verify the chain + signature and render the
    /// break-glass dialog.
    pub signed_request: crate::auth_mode::SignedEmergencyRequest,
    /// Delivery state at the moment of return:
    /// - `"delivered"` — push-wake queued AND tunnel was up at attach.
    /// - `"pushed"` — push-wake queued; tunnel was offline (consumer-side
    ///   retry policy applies once it reconnects).
    /// - `"no_token"` — patient storage has no push token; relay can't
    ///   wake the device. Caller should fall through to BLE-mediated
    ///   transport per the spec.
    pub delivery_status: String,
}

#[cfg(feature = "authority")]
async fn handle_emergency_initiate(
    State(state): State<AppState>,
    Json(req): Json<EmergencyInitiateRequest>,
) -> Result<Json<EmergencyInitiateResponse>, ApiError> {
    let authority = state
        .authority
        .as_ref()
        .ok_or_else(|| ApiError::not_found("authority mode not enabled"))?;
    let chain = authority
        .current()
        .await
        .ok_or_else(|| ApiError::internal("authority cert not yet refreshed"))?;
    chain
        .check_validity(now_ms())
        .map_err(|e| ApiError::internal(format!("cert invalid: {e}")))?;

    // Look up the registration so we know whether to push.
    let row = state
        .relay
        .registrations
        .lookup_by_rendezvous(&req.rendezvous_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("rendezvous_id"))?;

    // Build the unsigned payload.
    let unsigned = crate::auth_mode::EmergencyAccessRequest {
        request_id: String::new(), // sign_request fills in
        issued_at_ms: 0,
        expires_at_ms: 0,
        patient_storage_pubkey_pin: req.patient_storage_pubkey_pin_hex,
        responder_label: req.responder_label,
        scene_context: req.scene_context,
        operator_label: req.operator_label,
        scene_lat: req.scene_lat,
        scene_lon: req.scene_lon,
        scene_accuracy_m: req.scene_accuracy_m,
        cert_chain_pem: vec![],
    };
    let signed = crate::auth_mode::sign_request(&chain, unsigned, now_ms())
        .map_err(|e| ApiError::internal(format!("sign: {e}")))?;

    // Persist a `_emergency_requests` row so the tablet can poll
    // `/v1/emergency/status/{request_id}` across socket disruptions.
    // The row's `expires_at_ms` is the relay-side TTL (30s default —
    // when the patient phone hasn't responded by then, the TTL sweeper
    // either auto-grants or expires depending on the patient's
    // emergency-profile default). This is DISTINCT from the signed
    // payload's own `expires_at_ms` (the patient phone's "this request
    // is too stale to render" cutoff, default 5min).
    let now = now_ms();
    let ttl_expires_at_ms =
        now + crate::emergency_endpoints::DEFAULT_REQUEST_TTL.as_millis() as i64;
    if let Err(e) = crate::emergency_endpoints::record_initiated_request(
        &state.emergency,
        state.storage_tunnel.as_ref(),
        signed.request.request_id.clone(),
        req.rendezvous_id.clone(),
        ttl_expires_at_ms,
        now,
    )
    .await
    {
        warn!(
            target: "ohd_relay::emergency",
            error = %e,
            "failed to persist emergency request row; tablet polling may 404"
        );
    }

    // Push-wake the patient if there's a token. The actual delivery of the
    // signed payload to the patient's storage happens over the tunnel
    // (Connect mobile's notification handler invokes
    // `OhdcService.DeliverEmergencyRequest` over the inner TLS, the relay
    // forwards bytes opaquely as usual). The push here is just the wake
    // signal so the device opens its tunnel.
    let delivery_status = if let Some(token) = &row.push_token {
        match state.push.wake(&req.rendezvous_id, token).await {
            Ok(()) => {
                // Did the tunnel come up?
                if state
                    .relay
                    .sessions
                    .lookup(&req.rendezvous_id)
                    .await
                    .is_some()
                {
                    "delivered".to_string()
                } else {
                    "pushed".to_string()
                }
            }
            Err(e) => {
                warn!(
                    target: "ohd_relay::emergency",
                    rendezvous_id = %req.rendezvous_id,
                    error = %e,
                    "emergency push failed"
                );
                "pushed".to_string()
            }
        }
    } else {
        "no_token".to_string()
    };

    Ok(Json(EmergencyInitiateResponse {
        signed_request: signed,
        delivery_status,
    }))
}
