//! Remote-access share responder — the uniffi surface of CORD data-link
//! Phase 4d.
//!
//! `cord/spec/data-link.md` "Activating remote access" + "The phone-side
//! share responder": OHD Connect, for a share with remote access enabled,
//! registers a per-share relay rendezvous and runs a background responder
//! that serves a remote consumer (CORD) share-scoped MCP through the relay
//! tunnel.
//!
//! This module wraps [`ohd_relay_client::responder`] for Kotlin / Swift:
//!
//! - [`generate_storage_identity_key`] mints the storage's long-lived
//!   Ed25519 identity key. Connect persists it (Keystore-backed prefs) and
//!   passes the hex back on every call — it is the key that mints the
//!   inner-TLS cert, so its SPKI hash is the share artifact's `pin`.
//! - [`OhdStorage::register_remote_share`] performs the real relay
//!   `POST /v1/register` and returns the rendezvous id + credential + pin.
//! - [`OhdStorage::start_share_responder`] spawns the background tunnel +
//!   inner-TLS + scoped-MCP responder and hands back a
//!   [`ShareResponderHandle`]; dropping or `stop()`-ing it tears the
//!   responder down (used when the user disables remote access).
//!
//! The scoped-MCP-over-inner-TLS responder logic itself lives in
//! `ohd-relay-client`'s `responder` module and is fully real — not stubbed.
//! Relay registration is real. The only piece this binding cannot exercise
//! without a running relay is an *end-to-end live tunnel*; the responder
//! starts and dials regardless, reconnecting with backoff until a relay
//! answers.

use std::sync::{Arc, Mutex};

use ohd_relay_client::responder::{
    register_share_rendezvous, ShareRendezvous, ShareResponder,
};
use tokio::runtime::Runtime;
use tokio::sync::watch;

use crate::{core, OhdError, OhdStorage, Result};

// ===========================================================================
// Identity key
// ===========================================================================

/// Generate a fresh storage identity key — a long-lived Ed25519 keypair in
/// PKCS#8 DER form, returned hex-encoded.
///
/// Per `relay/spec/relay-protocol.md` "storage identity key + cert
/// pinning": each storage instance has one identity key, generated on first
/// launch. It signs the self-signed inner-TLS cert; the SHA-256 of its SPKI
/// is the `pin` baked into every share link. Connect generates this once
/// and persists it (Keystore-bound `EncryptedSharedPreferences`); rotating
/// it invalidates every outstanding share link, so it is generated rarely
/// and deliberately.
#[uniffi::export]
pub fn generate_storage_identity_key() -> Result<String> {
    crate::android_panic::install();
    let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).map_err(|e| {
        OhdError::Internal {
            code: "INTERNAL".into(),
            message: format!("generate identity key: {e}"),
        }
    })?;
    Ok(hex::encode(kp.serialize_der()))
}

// ===========================================================================
// DTOs
// ===========================================================================

/// The per-share relay rendezvous, as returned by
/// [`OhdStorage::register_remote_share`].
///
/// Connect builds the `ohd://share/...` artifact from these fields (the
/// `ShareLink` builder) and passes the whole record back into
/// [`OhdStorage::start_share_responder`] to bring the responder up.
#[derive(Debug, Clone, uniffi::Record)]
pub struct RemoteShareDto {
    /// Opaque per-share rendezvous id issued by the relay — the
    /// `<rendezvous_id>` of `ohd://share/<rendezvous_id>?...`.
    pub rendezvous_id: String,
    /// The relay's public URL for this rendezvous.
    pub rendezvous_url: String,
    /// The `long_lived_credential` authenticating subsequent tunnel opens.
    /// Sensitive — Connect keeps it out of the share artifact.
    pub long_lived_credential: String,
    /// SHA-256 of the storage identity cert's SPKI, base64url-no-pad — the
    /// `pin=` parameter of the share link, the cert-pinning trust anchor.
    pub spki_pin_b64url: String,
}

// ===========================================================================
// Responder handle
// ===========================================================================

/// A running share responder. Returned by
/// [`OhdStorage::start_share_responder`]; the responder maintains the relay
/// tunnel and answers scoped MCP for as long as this handle is alive.
///
/// Connect keeps one handle per share with remote access enabled. Disabling
/// remote access calls [`ShareResponderHandle::stop`]; dropping the handle
/// has the same effect (the `Drop` impl flips the shutdown signal).
#[derive(uniffi::Object)]
pub struct ShareResponderHandle {
    /// Flipping this to `true` unwinds the tunnel client cleanly.
    shutdown: watch::Sender<bool>,
    /// Background multi-thread runtime the responder task runs on. Kept
    /// alive by the handle; dropped (and thus shut down) with it.
    runtime: Mutex<Option<Runtime>>,
}

#[uniffi::export]
impl ShareResponderHandle {
    /// Stop the responder: signal shutdown, deregister the tunnel, and tear
    /// down the background runtime. Idempotent.
    pub fn stop(&self) {
        let _ = self.shutdown.send(true);
        if let Ok(mut guard) = self.runtime.lock() {
            // Dropping the runtime joins the responder task. Use a
            // background shutdown so a slow tunnel teardown can't block the
            // UI thread that called `stop()`.
            if let Some(rt) = guard.take() {
                rt.shutdown_background();
            }
        }
    }

    /// `true` while the responder is still serving (shutdown not signalled).
    pub fn is_running(&self) -> bool {
        !*self.shutdown.borrow()
    }
}

impl Drop for ShareResponderHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

// ===========================================================================
// OhdStorage surface
// ===========================================================================

#[uniffi::export]
impl OhdStorage {
    /// Register a per-share relay rendezvous — the network half of
    /// "activate remote access" (`cord/spec/data-link.md`).
    ///
    /// Performs the real relay `POST /v1/register`, scoped to this share:
    /// each share gets its own rendezvous, so disabling one share's remote
    /// access never disturbs another. Returns the rendezvous id, credential
    /// and the SPKI `pin` for the share artifact.
    ///
    /// - `relay_origin` — the relay's HTTP origin, e.g.
    ///   `https://relay.ohd.dev`.
    /// - `identity_key_hex` — the storage identity key from
    ///   [`generate_storage_identity_key`].
    /// - `share_label` — optional friendly label for the relay's listing.
    ///
    /// `grant_ulid` is accepted for symmetry / validation (the rendezvous
    /// is per-share); it is parsed so a bad ULID fails fast here rather than
    /// at responder-start.
    pub fn register_remote_share(
        &self,
        grant_ulid: String,
        relay_origin: String,
        identity_key_hex: String,
        share_label: Option<String>,
    ) -> Result<RemoteShareDto> {
        crate::android_panic::install();
        // Validate the grant exists before paying for a network round trip.
        let _ = self.grant_id_for(&grant_ulid)?;
        let identity_key = crate::hex_decode(&identity_key_hex)?;
        let user_ulid_hex = hex::encode(self.inner.user_ulid());

        let rendezvous = block_on(register_share_rendezvous(
            &relay_origin,
            &user_ulid_hex,
            &identity_key,
            share_label,
        ))
        .map_err(|e| OhdError::Internal {
            code: "RELAY_REGISTER_FAILED".into(),
            message: format!("relay registration failed: {e}"),
        })?;

        Ok(RemoteShareDto {
            rendezvous_id: rendezvous.rendezvous_id,
            rendezvous_url: rendezvous.rendezvous_url,
            long_lived_credential: rendezvous.long_lived_credential,
            spki_pin_b64url: rendezvous.spki_pin_b64url,
        })
    }

    /// Start the background share responder for `grant_ulid`.
    ///
    /// Spawns the relay tunnel client + inner-TLS server + scoped-MCP
    /// responder on a background runtime. The responder stays up — keeping
    /// the share reachable asynchronously — until the returned handle is
    /// stopped or dropped.
    ///
    /// - `share` — the [`RemoteShareDto`] from
    ///   [`Self::register_remote_share`].
    /// - `relay_tunnel_url` — `host:port` of the relay's QUIC tunnel
    ///   endpoint (its `--quic-tunnel-listen` address).
    /// - `identity_key_hex` — the same identity key registration used; it
    ///   mints the inner-TLS cert the consumer pins.
    /// - `allow_insecure_dev` — accept any relay QUIC cert (dev / tests).
    pub fn start_share_responder(
        &self,
        grant_ulid: String,
        share: RemoteShareDto,
        relay_tunnel_url: String,
        identity_key_hex: String,
        allow_insecure_dev: bool,
    ) -> Result<Arc<ShareResponderHandle>> {
        crate::android_panic::install();
        let grant_id = self.grant_id_for(&grant_ulid)?;
        let identity_key = crate::hex_decode(&identity_key_hex)?;

        let rendezvous = ShareRendezvous {
            rendezvous_id: share.rendezvous_id,
            rendezvous_url: share.rendezvous_url,
            long_lived_credential: share.long_lived_credential,
            spki_pin_b64url: share.spki_pin_b64url,
        };

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| OhdError::Internal {
                code: "INTERNAL".into(),
                message: format!("build responder runtime: {e}"),
            })?;

        let responder = ShareResponder::new(
            self.storage_arc(),
            grant_id,
            identity_key,
            &rendezvous,
            relay_tunnel_url,
            allow_insecure_dev,
        );

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        runtime.spawn(async move {
            if let Err(err) = responder.serve(shutdown_rx).await {
                tracing::warn!(?err, "share responder exited with error");
            }
        });

        Ok(Arc::new(ShareResponderHandle {
            shutdown: shutdown_tx,
            runtime: Mutex::new(Some(runtime)),
        }))
    }
}

impl OhdStorage {
    /// Resolve a Crockford grant ULID to its internal grant id.
    fn grant_id_for(&self, grant_ulid: &str) -> Result<i64> {
        let ulid_bytes = core::ulid::parse_crockford(grant_ulid).map_err(OhdError::from)?;
        self.inner
            .with_conn(|conn| core::grants::grant_id_by_ulid(conn, &ulid_bytes))
            .map_err(Into::into)
    }
}

/// Run a future to completion on a transient current-thread runtime — used
/// for the one-shot relay registration call, which has no long-lived
/// runtime of its own.
fn block_on<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build current-thread runtime")
        .block_on(fut)
}
