//! Raw QUIC bidirectional-stream tunnel mode.
//!
//! # Why a separate transport
//!
//! The relay's WebSocket-over-HTTP/2 tunnel works fine for desktop / wired
//! clients but loses its connection across mobile network handoffs (cellular
//! ↔ WiFi). Each handoff invalidates the underlying TCP socket; the relay
//! has to push-wake the device and the storage redials. Recovery is
//! O(1–3 seconds) — long enough that interactive consumers see stalls.
//!
//! Raw QUIC has **native connection migration** (RFC 9000 §9): on a path
//! change the QUIC stack revalidates the new path with PATH_CHALLENGE /
//! PATH_RESPONSE and keeps the connection alive without dropping streams.
//! For the storage→relay long-lived tunnel, this is the correct shape.
//! The relay tunnel carries opaque ciphertext (TLS terminates at storage
//! and consumer; the relay sees DATA-frame ciphertext only), so the
//! HTTP-framing overhead a WS-over-HTTP/3 mode would add buys nothing.
//!
//! WS-over-HTTP/3 (RFC 9220) also depends on h3 extended-CONNECT which
//! the `h3 0.0.x` crate doesn't yet expose to axum cleanly; raw QUIC
//! sidesteps that dependency entirely.
//!
//! The original WS-over-HTTP/2 tunnel stays as a fallback for networks
//! that block UDP/443 (some corporate proxies). When both transports are
//! enabled, storage prefers raw QUIC and falls back on UDP-block
//! detection.
//!
//! # Wire shape
//!
//! ## Handshake (stream 0 — first bidi stream the client opens)
//!
//! Client → server, big-endian. All fields length-prefixed so credentials
//! and rendezvous-ids of any reasonable size fit:
//!
//! ```text
//! [u8 version = 0x01]
//! [u8 cred_len]                       // 1..=HANDSHAKE_MAX_CRED_LEN
//! [cred_len bytes credential]         // ASCII long_lived_credential
//! [u16 token_len BE]                  // 1..=256
//! [token_len bytes registration_token] // ASCII rendezvous_id
//! ```
//!
//! Server → client:
//!
//! ```text
//! [u8 ack_status]   // 0x00 OK, 0x01 REGISTRATION_REJECTED
//! [16 bytes session-base-id]   // random; storage echoes it in subsequent stream prefixes for sanity
//! ```
//!
//! On reject, the server closes the connection with code
//! [`CloseCode::REGISTRATION_REJECTED`] (1) after writing the ack.
//!
//! After the handshake the same stream stays open as the **control
//! channel** (heartbeats, deregister hints, error frames). QUIC's PING is
//! handled by the transport stack; we layer one application-level
//! heartbeat for end-to-end liveness.
//!
//! ## Per-session streams (one bidi stream per consumer attach)
//!
//! The relay opens a *new* bidi stream toward the storage when a consumer
//! attaches. The relay writes a session prefix:
//!
//! ```text
//! [u8 = SESSION_OPEN (0x01)]
//! [u32 BE session_id]
//! ```
//!
//! Then the stream is opaque: TunnelFrame bytes flow in both directions
//! (relay→storage carries DATA / CLOSE etc. coming from the consumer;
//! storage→relay carries the storage's responses). Either side
//! `finish()`'s the stream to close cleanly; `reset()` for errors.
//!
//! ## Heartbeats
//!
//! On the control channel:
//!
//! ```text
//! [u8 = HEARTBEAT (0x02)]
//! [u64 BE timestamp_ms]
//! ```
//!
//! Replied with the same shape. Sent every 60 s by default; 3 misses tear
//! down the connection. (QUIC's PATH_CHALLENGE handles migration without
//! this; the application heartbeat catches dead peers.)
//!
//! # ALPN + endpoint isolation
//!
//! ALPN protocol identifier: `b"ohd-tnl1"`. We bind the tunnel listener
//! on a **separate UDP port** from the HTTP/3 REST listener. While quinn
//! supports multi-ALPN dispatch on a single endpoint, routing the
//! accepted connection to the right handler still requires inspecting
//! `connection.handshake_data()` post-handshake — which adds a coupling
//! between two otherwise-independent listeners. Separate ports keep the
//! two surfaces operationally and code-wise distinct (HTTP/3 on whatever
//! the operator picks for `--http3-listen`; tunnel on
//! `--quic-tunnel-listen`, default off).

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use bytes::{Bytes, BytesMut};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::sync::{mpsc, watch, Mutex, RwLock};
use tokio::time::Instant;
use tracing::{debug, info, warn};

use crate::frame::{FrameType, TunnelFrame};
use crate::state::{now_ms, RelayState};

// ---------------------------------------------------------------------------
// Wire constants
// ---------------------------------------------------------------------------

/// ALPN identifier carried in the QUIC handshake. 8 ASCII bytes.
pub const TUNNEL_ALPN: &[u8] = b"ohd-tnl1";

/// Currently-supported handshake protocol version.
pub const HANDSHAKE_VERSION: u8 = 0x01;

/// Reserved slot on-wire for the registration credential. We use a
/// length-prefixed string so credentials of any reasonable size fit
/// without truncation. The relay's [`crate::server::generate_credential`]
/// emits ~52-char base32 strings (26 random bytes → 52 chars unpadded).
/// We cap at 128 bytes — anything longer is almost certainly a probe.
pub const HANDSHAKE_MAX_CRED_LEN: usize = 128;

/// Marker on a per-session stream's first byte.
pub const STREAM_TAG_SESSION_OPEN: u8 = 0x01;

/// Marker for a control-channel heartbeat frame.
pub const CONTROL_TAG_HEARTBEAT: u8 = 0x02;

/// Default heartbeat interval (control channel).
pub const DEFAULT_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

/// Maximum consecutive missed heartbeats before tearing down.
pub const MAX_MISSED_HEARTBEATS: u32 = 3;

/// Per-stream chunk size for forwarding consumer↔storage bytes through the
/// per-session bidi stream pair.
pub const FORWARD_CHUNK_SIZE: usize = 32 * 1024;

/// Application-level QUIC close codes the tunnel uses.
pub mod close_code {
    /// Normal shutdown (operator-initiated, peer closed cleanly).
    pub const NORMAL: u32 = 0;
    /// Handshake validated against [`crate::state::RegistrationTable`] and
    /// failed (no such rendezvous_id, bad credential, expired entry, etc.).
    pub const REGISTRATION_REJECTED: u32 = 1;
    /// Heartbeat-timeout triggered teardown.
    pub const HEARTBEAT_TIMEOUT: u32 = 2;
    /// Malformed handshake (bad version, truncated bytes).
    pub const BAD_HANDSHAKE: u32 = 3;
    /// Internal error (resource exhaustion, downstream channel error).
    pub const INTERNAL: u32 = 4;
}

// ---------------------------------------------------------------------------
// Public state alias
// ---------------------------------------------------------------------------

/// State the QUIC tunnel handler needs from the relay. We take the full
/// [`RelayState`] (registrations + sessions + pairings) but in v1 only the
/// registration / session sub-tables are touched.
pub type TunnelState = RelayState;

// ---------------------------------------------------------------------------
// Public listener entry-point
// ---------------------------------------------------------------------------

/// Build a `quinn::ServerConfig` advertising the tunnel ALPN.
pub fn server_config(
    cert_chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<quinn::ServerConfig> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut tls = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .context("build rustls::ServerConfig")?;
    tls.max_early_data_size = u32::MAX;
    tls.alpn_protocols = vec![TUNNEL_ALPN.to_vec()];
    let quic_crypto: quinn::crypto::rustls::QuicServerConfig = tls
        .try_into()
        .context("rustls::ServerConfig → QuicServerConfig")?;
    let mut server = quinn::ServerConfig::with_crypto(Arc::new(quic_crypto));

    // Tune transport for long-lived tunnels:
    // - Keep-alive at the QUIC layer in case the application heartbeat is
    //   slow to detect; cuts down on wakeups vs. PING-bombing.
    // - Allow the storage (the QUIC client) to open lots of bidi streams
    //   — each is the handshake stream + one per session pair when we
    //   want to push storage→relay session state. The relay opens
    //   server-initiated bidi streams for each consumer attach; that's
    //   gated by the *client's* max_concurrent_bidi_streams setting, so
    //   this server-side config controls the reverse direction.
    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(Duration::from_secs(30)));
    transport.max_idle_timeout(Some(
        quinn::IdleTimeout::try_from(Duration::from_secs(120))
            .context("max_idle_timeout out of range")?,
    ));
    transport.max_concurrent_bidi_streams(quinn::VarInt::from_u32(256));
    server.transport_config(Arc::new(transport));

    Ok(server)
}

/// Run the tunnel listener until `shutdown` flips to `true`. Blocks on
/// the accept loop; spawn it as a tokio task.
pub async fn serve_quic_tunnel(
    addr: SocketAddr,
    cert_chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
    state: Arc<TunnelState>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let qcfg = server_config(cert_chain, key)?;
    let endpoint =
        quinn::Endpoint::server(qcfg, addr).context("quinn::Endpoint::server (tunnel)")?;
    info!(
        target: "ohd_relay::quic_tunnel",
        %addr,
        alpn = std::str::from_utf8(TUNNEL_ALPN).unwrap_or("(non-utf8)"),
        "ohd-relay raw QUIC tunnel listening"
    );

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                // Err = sender dropped; either case is reason to exit the
                // accept loop. Without the Err arm, dropping the Sender
                // turns this into a tight-spinning loop.
                if changed.is_err() || *shutdown.borrow() {
                    break;
                }
            }
            incoming = endpoint.accept() => {
                let Some(incoming) = incoming else { break };
                let st = state.clone();
                let sd = shutdown.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_connection(incoming, st, sd).await {
                        warn!(target: "ohd_relay::quic_tunnel", ?err, "tunnel connection ended with error");
                    }
                });
            }
        }
    }
    info!(target: "ohd_relay::quic_tunnel", "tunnel listener shutting down");
    endpoint.close(close_code::NORMAL.into(), b"shutdown");
    endpoint.wait_idle().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Per-connection lifecycle
// ---------------------------------------------------------------------------

/// Per-connection handler. Public for integration tests that drive the
/// accept loop manually (they need to pin the bind address rather than
/// pre-binding-and-dropping a UDP socket — the latter races with parallel
/// tests).
#[doc(hidden)]
pub async fn handle_connection_for_test(
    incoming: quinn::Incoming,
    state: Arc<TunnelState>,
    shutdown: watch::Receiver<bool>,
) -> Result<()> {
    handle_connection(incoming, state, shutdown).await
}

async fn handle_connection(
    incoming: quinn::Incoming,
    state: Arc<TunnelState>,
    shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let conn = incoming.await.context("quinn handshake")?;
    let remote = conn.remote_address();
    debug!(target: "ohd_relay::quic_tunnel", %remote, "new tunnel handshake");

    // The first bidi stream the client opens is the control / handshake
    // channel. We accept it, run the handshake, then keep it open as the
    // control channel for the connection's lifetime.
    let (mut ctrl_send, mut ctrl_recv) = conn
        .accept_bi()
        .await
        .context("accept handshake bi-stream")?;

    let handshake = match read_handshake(&mut ctrl_recv).await {
        Ok(h) => h,
        Err(err) => {
            warn!(target: "ohd_relay::quic_tunnel", ?err, "malformed handshake; closing");
            // Best-effort write a reject and close.
            let _ = write_handshake_ack(&mut ctrl_send, AckStatus::Reject, &[0u8; 16]).await;
            conn.close(close_code::BAD_HANDSHAKE.into(), b"BAD_HANDSHAKE");
            return Err(err);
        }
    };

    // Validate against the registration table.
    let auth_ok = validate_registration(&state, &handshake).await;
    let session_base_id = random_16();
    if !auth_ok {
        let _ = write_handshake_ack(&mut ctrl_send, AckStatus::Reject, &session_base_id).await;
        // Give the client a moment to read the ack before closing.
        let _ = ctrl_send.finish();
        conn.close(
            close_code::REGISTRATION_REJECTED.into(),
            b"REGISTRATION_REJECTED",
        );
        warn!(
            target: "ohd_relay::quic_tunnel",
            rendezvous_id = %handshake.rendezvous_id,
            "tunnel handshake rejected"
        );
        return Ok(());
    }
    write_handshake_ack(&mut ctrl_send, AckStatus::Ok, &session_base_id).await?;

    info!(
        target: "ohd_relay::quic_tunnel",
        rendezvous_id = %handshake.rendezvous_id,
        %remote,
        "tunnel handshake accepted"
    );

    // Build a TunnelEndpoint analog wired to QUIC. Reuses the existing
    // SessionTable so the consumer-attach (HTTP/2 WS) path can attach to a
    // QUIC-backed tunnel transparently.
    let tunnel = QuicTunnel::new(handshake.rendezvous_id.clone());
    state
        .registrations
        .update_endpoint(&handshake.rendezvous_id, true, now_ms())
        .await
        .ok();

    // Register a TunnelEndpoint pointing at our outbound queue so the
    // consumer-attach handler can `endpoint.outbound_tx.send(...)` and the
    // tunnel writer task here picks it up.
    let endpoint_for_attach = tunnel.bridge_endpoint();
    state
        .sessions
        .register_tunnel(endpoint_for_attach.clone())
        .await;

    // Cleanup helper.
    let cleanup_state = state.clone();
    let rendezvous_for_cleanup = handshake.rendezvous_id.clone();
    let cleanup = move || {
        let st = cleanup_state.clone();
        let rid = rendezvous_for_cleanup.clone();
        tokio::spawn(async move {
            if let Some(ep) = st.sessions.deregister_tunnel(&rid).await {
                ep.drain_all_sessions().await;
            }
            let _ = st.registrations.update_endpoint(&rid, false, now_ms()).await;
        });
    };

    // Drive the connection. Three concurrent tasks:
    // 1. Control-channel reader (handles heartbeats + control messages)
    // 2. Outbound-frame dispatcher: drains the TunnelEndpoint's outbound_tx
    //    and either (a) writes DATA bytes to the right per-session stream
    //    or (b) handles OPEN by spawning a new per-session stream pair.
    // 3. Heartbeat sender (tick + watchdog)
    let conn_for_streams = conn.clone();
    let tunnel_arc = Arc::new(tunnel);
    let outbound_dispatch = {
        let tunnel = tunnel_arc.clone();
        let conn = conn_for_streams.clone();
        let mut shutdown = shutdown.clone();
        tokio::spawn(async move {
            outbound_pump(tunnel, conn, &mut shutdown).await;
        })
    };

    let heartbeat_state = HeartbeatState::new();
    let ctrl_reader = {
        let hb = heartbeat_state.clone();
        let send = Arc::new(Mutex::new(ctrl_send));
        let send_for_reader = send.clone();
        let mut shutdown = shutdown.clone();
        let task = tokio::spawn(async move {
            ctrl_reader_loop(ctrl_recv, send_for_reader, hb, &mut shutdown).await;
        });
        (task, send)
    };
    let (ctrl_reader_task, ctrl_send_arc) = ctrl_reader;

    let heartbeat_pulse = {
        let hb = heartbeat_state.clone();
        let send = ctrl_send_arc.clone();
        let conn = conn_for_streams.clone();
        let mut shutdown = shutdown.clone();
        tokio::spawn(async move {
            heartbeat_pulse_loop(send, hb, conn, &mut shutdown).await;
        })
    };

    // Wait for any of: connection closed by peer, shutdown signal, control
    // reader exit, heartbeat watchdog tripping.
    let mut shutdown_wait = shutdown.clone();
    tokio::select! {
        _ = conn.closed() => {
            debug!(target: "ohd_relay::quic_tunnel", rendezvous_id = %handshake.rendezvous_id, "peer closed");
        }
        _ = shutdown_wait.changed() => {
            if *shutdown_wait.borrow() {
                debug!(target: "ohd_relay::quic_tunnel", "shutdown signaled; closing tunnel");
                conn.close(close_code::NORMAL.into(), b"shutdown");
            }
        }
        _ = ctrl_reader_task => {
            debug!(target: "ohd_relay::quic_tunnel", "control reader exited");
        }
    }

    outbound_dispatch.abort();
    heartbeat_pulse.abort();
    cleanup();
    info!(
        target: "ohd_relay::quic_tunnel",
        rendezvous_id = %handshake.rendezvous_id,
        "tunnel down"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Handshake
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Handshake {
    rendezvous_id: String,
    credential: String,
}

async fn read_handshake(stream: &mut quinn::RecvStream) -> Result<Handshake> {
    let mut version_byte = [0u8; 1];
    stream
        .read_exact(&mut version_byte)
        .await
        .context("read handshake version")?;
    if version_byte[0] != HANDSHAKE_VERSION {
        anyhow::bail!("unsupported handshake version {:#x}", version_byte[0]);
    }
    let mut cred_len_byte = [0u8; 1];
    stream
        .read_exact(&mut cred_len_byte)
        .await
        .context("read cred_len")?;
    let cred_len = cred_len_byte[0] as usize;
    if cred_len == 0 || cred_len > HANDSHAKE_MAX_CRED_LEN {
        anyhow::bail!("handshake cred_len out of range: {cred_len}");
    }
    let mut cred = vec![0u8; cred_len];
    stream
        .read_exact(&mut cred)
        .await
        .context("read credential")?;
    let credential = String::from_utf8(cred).context("credential is not utf-8")?;

    let mut token_len_bytes = [0u8; 2];
    stream
        .read_exact(&mut token_len_bytes)
        .await
        .context("read token_len")?;
    let token_len = u16::from_be_bytes(token_len_bytes) as usize;
    if token_len == 0 || token_len > 256 {
        anyhow::bail!("handshake token_len out of range: {token_len}");
    }
    let mut token = vec![0u8; token_len];
    stream
        .read_exact(&mut token)
        .await
        .context("read handshake token")?;
    let rendezvous_id = String::from_utf8(token).context("token is not utf-8")?;
    Ok(Handshake {
        rendezvous_id,
        credential,
    })
}

#[derive(Debug, Clone, Copy)]
enum AckStatus {
    Ok,
    Reject,
}

async fn write_handshake_ack(
    stream: &mut quinn::SendStream,
    status: AckStatus,
    session_base_id: &[u8; 16],
) -> Result<()> {
    let mut buf = [0u8; 1 + 16];
    buf[0] = match status {
        AckStatus::Ok => 0x00,
        AckStatus::Reject => 0x01,
    };
    buf[1..].copy_from_slice(session_base_id);
    stream.write_all(&buf).await.context("write handshake ack")?;
    Ok(())
}

async fn validate_registration(state: &TunnelState, h: &Handshake) -> bool {
    let row = match state
        .registrations
        .lookup_by_rendezvous(&h.rendezvous_id)
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            warn!(
                target: "ohd_relay::quic_tunnel",
                rendezvous_id = %h.rendezvous_id,
                "unknown rendezvous_id at handshake"
            );
            return false;
        }
        Err(err) => {
            warn!(target: "ohd_relay::quic_tunnel", ?err, "registration lookup failed");
            return false;
        }
    };
    let presented = sha256_32(h.credential.as_bytes());
    constant_time_eq_32(&row.long_lived_credential_hash, &presented)
}

// ---------------------------------------------------------------------------
// Outbound dispatcher: relay → storage
// ---------------------------------------------------------------------------

/// State + plumbing for one accepted QUIC tunnel.
struct QuicTunnel {
    rendezvous_id: String,
    /// The bridge `TunnelEndpoint` we register in [`SessionTable`] so the
    /// existing consumer-attach handler can talk to us.
    endpoint: crate::session::TunnelEndpoint,
    /// Receiver side of `endpoint.outbound_tx`. Drained by the outbound
    /// dispatcher.
    outbound_rx: Arc<Mutex<Option<mpsc::Receiver<TunnelFrame>>>>,
    /// Per-session per-direction QUIC SendStream handles (relay→storage).
    /// Keyed by `session_id`.
    streams: Arc<RwLock<HashMap<u32, Arc<Mutex<quinn::SendStream>>>>>,
}

impl QuicTunnel {
    fn new(rendezvous_id: String) -> Self {
        let (endpoint, outbound_rx) = crate::session::TunnelEndpoint::new(rendezvous_id.clone());
        Self {
            rendezvous_id,
            endpoint,
            outbound_rx: Arc::new(Mutex::new(Some(outbound_rx))),
            streams: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn bridge_endpoint(&self) -> crate::session::TunnelEndpoint {
        self.endpoint.clone()
    }
}

async fn outbound_pump(
    tunnel: Arc<QuicTunnel>,
    conn: quinn::Connection,
    shutdown: &mut watch::Receiver<bool>,
) {
    let mut rx = match tunnel.outbound_rx.lock().await.take() {
        Some(rx) => rx,
        None => {
            warn!(target: "ohd_relay::quic_tunnel", "outbound_rx already taken; pump exiting");
            return;
        }
    };

    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { break; }
            }
            maybe = rx.recv() => {
                let Some(frame) = maybe else { break };
                if let Err(err) = handle_outbound_frame(&tunnel, &conn, frame).await {
                    warn!(target: "ohd_relay::quic_tunnel", ?err, "outbound frame failed");
                    break;
                }
            }
        }
    }
    debug!(target: "ohd_relay::quic_tunnel", rendezvous_id = %tunnel.rendezvous_id, "outbound pump exiting");
}

async fn handle_outbound_frame(
    tunnel: &Arc<QuicTunnel>,
    conn: &quinn::Connection,
    frame: TunnelFrame,
) -> Result<()> {
    match frame.frame_type {
        FrameType::Open => {
            // New consumer attach. Open a fresh bidi stream toward storage,
            // write the SESSION_OPEN prefix, register the SendStream so
            // subsequent DATA frames for this session route to it, and
            // spawn a reader that pumps storage→relay bytes back into the
            // session's consumer-side channel.
            let session_id = frame.session_id;
            let (mut send, recv) = match conn.open_bi().await {
                Ok(pair) => pair,
                Err(err) => {
                    anyhow::bail!("open_bi failed: {err}");
                }
            };
            // Write [SESSION_OPEN][session_id BE u32].
            let mut prefix = [0u8; 5];
            prefix[0] = STREAM_TAG_SESSION_OPEN;
            prefix[1..5].copy_from_slice(&session_id.to_be_bytes());
            if let Err(err) = send.write_all(&prefix).await {
                anyhow::bail!("write session prefix: {err}");
            }
            // Optional: forward the OPEN payload (consumer's grant token
            // preview) inline to give the storage early auth context. We
            // emit a single TunnelFrame::open envelope so the storage's
            // existing frame parser can handle it uniformly.
            let open_envelope = TunnelFrame::open(session_id, frame.payload).encode()?;
            if let Err(err) = send.write_all(&open_envelope).await {
                anyhow::bail!("write OPEN envelope: {err}");
            }

            let send_arc = Arc::new(Mutex::new(send));
            tunnel
                .streams
                .write()
                .await
                .insert(session_id, send_arc.clone());

            // Reader task: storage→relay bytes for this session. We treat
            // every chunk as opaque bytes and shovel into the session's
            // attached_senders channel via the existing TunnelEndpoint
            // dispatch path. Storage emits TunnelFrame envelopes (DATA,
            // CLOSE, etc.); we decode each frame here and route by type.
            let endpoint = tunnel.endpoint.clone();
            tokio::spawn(async move {
                if let Err(err) = session_reader_loop(session_id, recv, endpoint).await {
                    debug!(
                        target: "ohd_relay::quic_tunnel",
                        session_id,
                        ?err,
                        "session reader exited"
                    );
                }
            });
        }
        FrameType::Data => {
            // Forward DATA bytes on the corresponding per-session stream.
            let session_id = frame.session_id;
            let send_arc = {
                let g = tunnel.streams.read().await;
                g.get(&session_id).cloned()
            };
            if let Some(send_arc) = send_arc {
                let bytes = frame.encode()?;
                let mut send = send_arc.lock().await;
                send.write_all(&bytes).await.context("write DATA")?;
            } else {
                debug!(
                    target: "ohd_relay::quic_tunnel",
                    session_id,
                    "DATA for unknown session; dropping"
                );
            }
        }
        FrameType::Close => {
            let session_id = frame.session_id;
            let stream = tunnel.streams.write().await.remove(&session_id);
            if let Some(send_arc) = stream {
                let bytes = frame.encode()?;
                let mut send = send_arc.lock().await;
                let _ = send.write_all(&bytes).await;
                let _ = send.finish();
            }
        }
        FrameType::Hello | FrameType::Ping | FrameType::Pong => {
            // Control frames don't ride per-session streams; we emit an
            // application-level heartbeat on the control channel instead.
            // Nothing to do for HELLO/PING/PONG over raw QUIC — the
            // bridging endpoint never produces these directly.
        }
        FrameType::OpenAck | FrameType::OpenNack | FrameType::WindowUpdate => {
            // These flow storage→relay only; relay never originates them.
            let ft = frame.frame_type;
            debug!(
                target: "ohd_relay::quic_tunnel",
                ?ft,
                "unexpected outbound control frame from relay; dropping"
            );
        }
    }
    Ok(())
}

async fn session_reader_loop(
    session_id: u32,
    mut recv: quinn::RecvStream,
    endpoint: crate::session::TunnelEndpoint,
) -> Result<()> {
    let mut buf = BytesMut::with_capacity(FORWARD_CHUNK_SIZE * 2);
    loop {
        let mut chunk = vec![0u8; FORWARD_CHUNK_SIZE];
        match recv.read(&mut chunk).await? {
            Some(n) => {
                buf.extend_from_slice(&chunk[..n]);
            }
            None => break,
        }

        // Drain whole frames out of the buffer.
        loop {
            match TunnelFrame::decode_one(&buf) {
                Ok((frame, consumed)) => {
                    let _ = buf.split_to(consumed);
                    handle_storage_originated_frame(session_id, &endpoint, frame).await;
                }
                Err(crate::frame::FrameError::Truncated { .. }) => break,
                Err(err) => {
                    warn!(
                        target: "ohd_relay::quic_tunnel",
                        session_id,
                        ?err,
                        "frame decode failed on per-session stream"
                    );
                    return Err(anyhow::anyhow!("frame decode: {err}"));
                }
            }
        }
    }
    if !buf.is_empty() {
        warn!(
            target: "ohd_relay::quic_tunnel",
            session_id,
            leftover_bytes = buf.len(),
            "session stream ended with partial frame"
        );
    }
    Ok(())
}

async fn handle_storage_originated_frame(
    session_id: u32,
    endpoint: &crate::session::TunnelEndpoint,
    frame: TunnelFrame,
) {
    use crate::frame::FrameType as F;
    // Storage sends DATA / OPEN_ACK / OPEN_NACK / CLOSE / etc. on the
    // session stream. We delegate to the same routing primitives as the
    // WS handler. The consumer-side `attached_senders` map lives in
    // `session.rs::attached_senders_for(endpoint)` and is shared with the
    // HTTP/2 WebSocket tunnel handler, so the consumer-attach flow doesn't
    // care which transport the storage is on.
    match frame.frame_type {
        F::Data => {
            let _ = forward_storage_to_consumer(endpoint, session_id, frame.payload).await;
        }
        F::OpenAck => {
            debug!(target: "ohd_relay::quic_tunnel", session_id, "OPEN_ACK from storage");
        }
        F::OpenNack | F::Close => {
            let senders = crate::session::attached_senders_for(endpoint);
            senders.write().await.remove(&session_id);
        }
        F::WindowUpdate | F::Ping | F::Pong => {
            // Advisory; ignore in v1.
        }
        F::Hello | F::Open => {
            // Storage shouldn't originate these per the spec.
            let ft = frame.frame_type;
            debug!(
                target: "ohd_relay::quic_tunnel",
                ?ft,
                "unexpected storage-originated control frame; ignoring"
            );
        }
    }
}

async fn forward_storage_to_consumer(
    endpoint: &crate::session::TunnelEndpoint,
    session_id: u32,
    payload: Bytes,
) -> Result<()> {
    let senders = crate::session::attached_senders_for(endpoint);
    let tx = {
        let g = senders.read().await;
        g.get(&session_id).cloned()
    };
    if let Some(tx) = tx {
        let _ = tx.send(payload).await;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Heartbeats / control channel
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct HeartbeatState {
    last_inbound_at: Arc<Mutex<Instant>>,
    misses: Arc<AtomicU32>,
}

impl HeartbeatState {
    fn new() -> Self {
        Self {
            last_inbound_at: Arc::new(Mutex::new(Instant::now())),
            misses: Arc::new(AtomicU32::new(0)),
        }
    }
    async fn note_inbound(&self) {
        *self.last_inbound_at.lock().await = Instant::now();
        self.misses.store(0, Ordering::SeqCst);
    }
}

async fn ctrl_reader_loop(
    mut recv: quinn::RecvStream,
    send: Arc<Mutex<quinn::SendStream>>,
    hb: HeartbeatState,
    shutdown: &mut watch::Receiver<bool>,
) {
    let mut buf = BytesMut::with_capacity(64);
    let mut chunk = vec![0u8; 256];
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { break; }
            }
            res = recv.read(&mut chunk) => {
                match res {
                    Ok(Some(n)) => buf.extend_from_slice(&chunk[..n]),
                    Ok(None) => break,
                    Err(err) => {
                        debug!(target: "ohd_relay::quic_tunnel", ?err, "ctrl read err");
                        break;
                    }
                }
                while let Some(consumed) = try_parse_control_frame(&buf, &send, &hb).await {
                    let _ = buf.split_to(consumed);
                }
            }
        }
    }
}

async fn try_parse_control_frame(
    buf: &[u8],
    send: &Arc<Mutex<quinn::SendStream>>,
    hb: &HeartbeatState,
) -> Option<usize> {
    if buf.is_empty() {
        return None;
    }
    match buf[0] {
        CONTROL_TAG_HEARTBEAT => {
            // [tag = 0x02][u64 BE timestamp_ms]
            if buf.len() < 1 + 8 {
                return None;
            }
            let mut ts_bytes = [0u8; 8];
            ts_bytes.copy_from_slice(&buf[1..9]);
            // Echo back the same shape as ack.
            let mut reply = [0u8; 9];
            reply[0] = CONTROL_TAG_HEARTBEAT;
            reply[1..].copy_from_slice(&ts_bytes);
            let mut s = send.lock().await;
            let _ = s.write_all(&reply).await;
            hb.note_inbound().await;
            Some(9)
        }
        other => {
            warn!(target: "ohd_relay::quic_tunnel", tag = other, "unknown control tag; closing");
            None
        }
    }
}

async fn heartbeat_pulse_loop(
    send: Arc<Mutex<quinn::SendStream>>,
    hb: HeartbeatState,
    conn: quinn::Connection,
    shutdown: &mut watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(DEFAULT_HEARTBEAT_INTERVAL);
    ticker.tick().await; // skip the immediate fire
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { break; }
            }
            _ = ticker.tick() => {
                let last = *hb.last_inbound_at.lock().await;
                if last.elapsed() > DEFAULT_HEARTBEAT_INTERVAL * MAX_MISSED_HEARTBEATS {
                    let n = hb.misses.fetch_add(1, Ordering::SeqCst) + 1;
                    if n >= MAX_MISSED_HEARTBEATS {
                        warn!(
                            target: "ohd_relay::quic_tunnel",
                            "heartbeat watchdog tripped; closing connection"
                        );
                        conn.close(close_code::HEARTBEAT_TIMEOUT.into(), b"HEARTBEAT_TIMEOUT");
                        break;
                    }
                }
                let now_ts = now_ms() as u64;
                let mut buf = [0u8; 9];
                buf[0] = CONTROL_TAG_HEARTBEAT;
                buf[1..].copy_from_slice(&now_ts.to_be_bytes());
                let mut s = send.lock().await;
                if s.write_all(&buf).await.is_err() {
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn random_16() -> [u8; 16] {
    use rand::RngCore;
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    buf
}

fn sha256_32(input: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_ack_buffer_layout() {
        // Ensure the wire constants match documented sizes.
        assert!(HANDSHAKE_MAX_CRED_LEN >= 64); // base32 of 32 bytes ≈ 52
        assert_eq!(TUNNEL_ALPN.len(), 8);
    }

    #[test]
    fn close_codes_distinct() {
        let codes = [
            close_code::NORMAL,
            close_code::REGISTRATION_REJECTED,
            close_code::HEARTBEAT_TIMEOUT,
            close_code::BAD_HANDSHAKE,
            close_code::INTERNAL,
        ];
        let mut sorted = codes.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), codes.len());
    }

}
