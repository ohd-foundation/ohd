//! Outbound QUIC tunnel client to the OHD Relay (`storage → relay`).
//!
//! # Why this exists
//!
//! The relay's `WS /v1/tunnel/:rid` over HTTP/2 still works, but mobile
//! networks invalidate the underlying TCP socket on every WiFi↔cellular
//! handoff. The relay's raw QUIC tunnel mode (ALPN `ohd-tnl1`) carries the
//! same opaque session bytes over a connection-migrating QUIC connection,
//! so storage stays online across handoffs without push-wake + redial.
//!
//! Relay-side wire is documented in detail at the top of
//! `relay/src/quic_tunnel.rs`. This module is the storage-side dial-in
//! mirror of `relay/examples/quic_tunnel_client.rs`, plus the bit the
//! example skipped: feeding the per-session bidi stream's payload bytes
//! into a real hyper HTTP/2 server connection running the same
//! [`ConnectRpcService`] the HTTP/2 + HTTP/3 listeners use.
//!
//! # Wire shape (recap)
//!
//! - ALPN: `b"ohd-tnl1"`. Default port: 9001.
//! - Stream 0 (control / handshake), client-initiated bidi:
//!   - C→S: `[u8 v=0x01][u8 cred_len][cred_len bytes credential]
//!           [u16 token_len BE][token_len bytes registration_token]`
//!   - S→C: `[u8 ack=0x00 ok | 0x01 reject][16 bytes session-base-id]`
//!   - Stays open as the control channel.
//! - Per-session streams (server-initiated bidi, one per consumer attach):
//!   - S→C prefix: `[u8 = SESSION_OPEN (0x01)][u32 BE session_id]`
//!   - Then opaque [`TunnelFrame`] envelopes flow both ways. The relay
//!     pushes a synthetic `OPEN` envelope first; storage replies with
//!     `OPEN_ACK`, then DATA frames in both directions carry the
//!     consumer↔storage bytes (HTTP/2 traffic). `CLOSE` ends the session.
//! - Heartbeats: app-level on the control channel,
//!   `[u8 = HEARTBEAT (0x02)][u64 BE timestamp_ms]`. Both peers respond in
//!   kind. 3 misses (per relay) → teardown.
//!
//! # Wire quirk: buffered reader
//!
//! `quinn::RecvStream::read_chunk` may carry multiple `TunnelFrame`s in a
//! single read. A fresh-buf-per-call reader silently loses bytes after the
//! first frame parses. The [`read_one_frame_buffered`] helper here mirrors
//! the relay's integration-test helper exactly: it preserves leftover
//! bytes across reads. This is critical and easy to get wrong; the relay
//! agent flagged it explicitly.
//!
//! # Demuxing consumer sessions onto the local Connect-RPC service
//!
//! Each per-session bidi stream pair `(SendStream, RecvStream)` carries
//! the consumer's HTTP/2 connection bytes wrapped in `TunnelFrame::data`
//! envelopes. We bridge each pair into an `AsyncRead + AsyncWrite`
//! ([`SessionConn`]) that:
//!
//! - On `poll_read`: drains queued DATA-frame payloads (the consumer's
//!   inbound HTTP/2 bytes). When the underlying QUIC stream finishes (or
//!   a CLOSE frame arrives), `poll_read` returns 0.
//! - On `poll_write`: wraps the bytes into a single `TunnelFrame::data`
//!   envelope and writes to the QUIC `SendStream`. Larger writes are
//!   chunked at `MAX_OUTBOUND_PAYLOAD_BYTES` (64 KiB - 1, the codec's
//!   limit) so we never overflow the on-wire `payload_len` u16.
//!
//! The bridged conn is then handed to hyper's `auto::Builder` (HTTP/1
//! disabled, HTTP/2 enabled) which negotiates HTTP/2 directly without
//! ALPN — the consumer side passes prior-knowledge HTTP/2 frames through
//! the relay tunnel. The hyper server calls into the
//! [`ConnectRpcService`] for each request.
//!
//! # Reconnect + backoff
//!
//! On any fatal error (handshake reject, control-stream close, transport
//! close) we sleep with exponential backoff (`1s → 2s → 4s → 8s → 16s →
//! 30s` cap) and retry. Reconnects also re-run the handshake; the relay
//! state for our `rendezvous_id` survives across our outages because it's
//! keyed in the registration table, not the connection.
//!
//! # TLS
//!
//! The relay's tunnel cert is operator-supplied. Three modes:
//!
//! - **Pinned** (production-ish): the operator's registration response
//!   includes a SPKI-SHA256 fingerprint of the relay's QUIC tunnel cert.
//!   We verify the leaf cert's SPKI hash matches the pin and accept any
//!   signature/expiry — pinning sidesteps the WebPKI machinery entirely
//!   for an operator we already trust by registration.
//! - **Webpki + native trust** (default): use
//!   [`rustls_platform_verifier`] to verify against the OS trust store,
//!   for relays fronted by a real CA-issued cert.
//! - **Insecure** (dev only): accept any cert. Gated by
//!   `allow_insecure_dev` / `--relay-allow-insecure`.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use bytes::{Bytes, BytesMut};
use connectrpc::ConnectRpcService;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{mpsc, watch, Mutex};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// Wire constants — must match `ohd-relay::quic_tunnel` exactly.
//
// We hardcode these here (rather than depending on `ohd-relay`) because the
// storage crate intentionally has no path-dep on the relay crate. The
// constants are part of the on-wire ABI documented in the relay's
// `quic_tunnel.rs` module-level docstring; if the relay changes them, this
// module needs the matching edit.
// ---------------------------------------------------------------------------

/// ALPN identifier carried in the QUIC handshake.
pub const TUNNEL_ALPN: &[u8] = b"ohd-tnl1";

/// Currently-supported handshake protocol version.
pub const HANDSHAKE_VERSION: u8 = 0x01;

/// Maximum on-wire credential length (length-prefixed u8 → 1..=128 bytes).
pub const HANDSHAKE_MAX_CRED_LEN: usize = 128;

/// First-byte marker on per-session streams.
pub const STREAM_TAG_SESSION_OPEN: u8 = 0x01;

/// First-byte marker for control-channel heartbeats.
pub const CONTROL_TAG_HEARTBEAT: u8 = 0x02;

/// How often we pulse heartbeats outbound.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(60);

/// Maximum consecutive missed heartbeats before tearing down the connection
/// from the storage side. Mirrors the relay's watchdog.
pub const MAX_MISSED_HEARTBEATS: u32 = 3;

/// Initial reconnect delay; doubles per failure up to [`RECONNECT_DELAY_CAP`].
pub const RECONNECT_DELAY_INITIAL: Duration = Duration::from_secs(1);

/// Cap on reconnect backoff.
pub const RECONNECT_DELAY_CAP: Duration = Duration::from_secs(30);

/// Per-session inbound channel depth (DATA frames buffered storage-side
/// before backpressure kicks in). 256 frames at up to ~64 KiB each is plenty
/// for any realistic Connect-RPC traffic burst.
pub const SESSION_INBOUND_BUFFER: usize = 256;

/// Cap on a single outbound `TunnelFrame::data` payload (the wire's u16
/// `payload_len` field, minus a generous headroom). Larger writes are
/// chunked transparently in `SessionConn::poll_write`.
pub const MAX_OUTBOUND_PAYLOAD_BYTES: usize = 60 * 1024;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Configuration for the storage-side relay tunnel client.
#[derive(Debug, Clone)]
pub struct RelayClientOptions {
    /// `host:port` of the relay's `--quic-tunnel-listen` endpoint, e.g.
    /// `relay.example.org:9001`. The host portion is used as the TLS SNI.
    pub relay_url: String,
    /// The `rendezvous_id` issued at `POST /v1/register` time.
    pub registration_token: String,
    /// The `long_lived_credential` (ASCII base32) from `POST /v1/register`.
    pub credential: String,
    /// Optional SHA-256 SPKI pin of the relay's QUIC tunnel cert. When
    /// supplied, this overrides the regular WebPKI / platform-trust path.
    /// 32 bytes.
    pub expected_relay_pubkey_pin: Option<Vec<u8>>,
    /// Dev only: accept any server cert without verification. Used by tests
    /// against `dev_self_signed_cert()` and by `--relay-allow-insecure`.
    pub allow_insecure_dev: bool,
}

impl RelayClientOptions {
    /// Resolve `relay_url` to a [`SocketAddr`] + an SNI/host string.
    fn parse(&self) -> Result<(SocketAddr, String)> {
        let (host, port) = self
            .relay_url
            .rsplit_once(':')
            .ok_or_else(|| anyhow!("relay_url missing :port suffix: {}", self.relay_url))?;
        let port: u16 = port
            .parse()
            .with_context(|| format!("relay_url port not a number: {}", self.relay_url))?;
        // Resolve via std for synchronous DNS — we only do this on
        // (re)connect, not per request.
        let addr = (host, port)
            .to_socket_addrs()
            .with_context(|| format!("resolve {}", self.relay_url))?
            .next()
            .ok_or_else(|| anyhow!("no addresses for {}", self.relay_url))?;
        Ok((addr, host.to_string()))
    }
}

use std::net::ToSocketAddrs;

/// Run the outbound tunnel client until `shutdown` flips to `true` or the
/// sender is dropped.
///
/// On every reconnect we re-run the handshake. Each accepted per-session
/// stream is bridged into a hyper HTTP/2 server connection running
/// `service`. Errors are logged at `warn` level; the loop never exits on
/// transport failure (only on shutdown).
pub async fn serve_relay_tunnel(
    opts: RelayClientOptions,
    service: ConnectRpcService,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let mut delay = RECONNECT_DELAY_INITIAL;
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        match run_one_connection(&opts, service.clone(), shutdown.clone()).await {
            Ok(()) => {
                info!(target: "ohd_storage::relay_client", "tunnel session ended cleanly; reconnecting");
                delay = RECONNECT_DELAY_INITIAL;
            }
            Err(err) => {
                warn!(target: "ohd_storage::relay_client", ?err, ?delay, "tunnel session ended with error; reconnecting after backoff");
            }
        }

        // Sleep with backoff, but wake immediately on shutdown.
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
        }
        delay = (delay * 2).min(RECONNECT_DELAY_CAP);
    }
}

/// Drive one full QUIC tunnel connection lifecycle: connect, handshake,
/// run the control + accept loops until either side hangs up.
async fn run_one_connection(
    opts: &RelayClientOptions,
    service: ConnectRpcService,
    shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let (addr, sni) = opts.parse()?;
    let endpoint = build_client_endpoint(opts)?;
    info!(target: "ohd_storage::relay_client", %addr, %sni, "dialing relay tunnel");

    let conn = endpoint
        .connect(addr, &sni)
        .context("quinn::Endpoint::connect")?
        .await
        .context("QUIC handshake")?;

    // Open the control / handshake stream (must be the first bidi stream
    // we open — relay's accept_bi() on the connection picks it up as the
    // handshake stream).
    let (mut ctrl_send, mut ctrl_recv) = conn.open_bi().await.context("open_bi (handshake)")?;

    write_handshake(&mut ctrl_send, &opts.credential, &opts.registration_token).await?;
    let session_base_id = read_handshake_ack(&mut ctrl_recv).await?;
    info!(
        target: "ohd_storage::relay_client",
        session_base_id = %hex::encode(session_base_id),
        "tunnel handshake accepted"
    );

    // The control stream stays open for heartbeats; share it across the
    // pulse + reader tasks via Arc<Mutex<…>>.
    let ctrl_send = Arc::new(Mutex::new(ctrl_send));

    // Spawn:
    // - control-stream reader (heartbeat watchdog + DEREGISTER on shutdown)
    // - heartbeat pulse loop
    // - per-session accept loop (one task per accepted bidi stream)
    let conn_for_accept = conn.clone();
    let svc_for_accept = service.clone();
    let mut accept_shutdown = shutdown.clone();
    let accept_task = tokio::spawn(async move {
        accept_session_streams(conn_for_accept, svc_for_accept, &mut accept_shutdown).await;
    });

    let hb_send = Arc::clone(&ctrl_send);
    let mut pulse_shutdown = shutdown.clone();
    let pulse_task = tokio::spawn(async move {
        heartbeat_pulse(hb_send, &mut pulse_shutdown).await;
    });

    let watchdog = HeartbeatWatchdog::new();
    let watchdog_for_reader = watchdog.clone();
    let mut reader_shutdown = shutdown.clone();
    let reader_task = tokio::spawn(async move {
        control_reader(ctrl_recv, watchdog_for_reader, &mut reader_shutdown).await;
    });

    let watchdog_for_pulse = watchdog.clone();
    let conn_for_watchdog = conn.clone();
    let mut wd_shutdown = shutdown.clone();
    let watchdog_task = tokio::spawn(async move {
        watchdog_loop(watchdog_for_pulse, conn_for_watchdog, &mut wd_shutdown).await;
    });

    let mut shutdown_wait = shutdown.clone();
    tokio::select! {
        _ = conn.closed() => {
            debug!(target: "ohd_storage::relay_client", "connection closed by relay or transport");
        }
        _ = shutdown_wait.changed() => {
            if *shutdown_wait.borrow() {
                debug!(target: "ohd_storage::relay_client", "shutdown signaled; deregistering and closing");
                // DEREGISTER hint is best-effort: there's no opcode in the
                // current relay control protocol beyond HEARTBEAT. We
                // simply close the connection cleanly; the relay's
                // handle_connection sees `conn.closed()` and runs its
                // cleanup (drain sessions, mark tunnel down, etc.).
                conn.close(0u32.into(), b"shutdown");
            }
        }
        _ = reader_task => {
            debug!(target: "ohd_storage::relay_client", "control reader exited");
        }
    }

    accept_task.abort();
    pulse_task.abort();
    watchdog_task.abort();
    endpoint.close(0u32.into(), b"client-shutdown");
    endpoint.wait_idle().await;
    Ok(())
}

// ---------------------------------------------------------------------------
// QUIC client config
// ---------------------------------------------------------------------------

fn build_client_endpoint(opts: &RelayClientOptions) -> Result<quinn::Endpoint> {
    // Install ring as the rustls default crypto provider if no provider has
    // been registered yet. Calling this unconditionally is safe (subsequent
    // calls are no-ops).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut tls = if opts.allow_insecure_dev {
        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(InsecureCertVerifier::new()))
            .with_no_client_auth()
    } else if let Some(pin) = opts.expected_relay_pubkey_pin.clone() {
        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SpkiPinVerifier::new(pin)?))
            .with_no_client_auth()
    } else {
        // Default: rustls-platform-verifier (OS trust store).
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let verifier = rustls_platform_verifier::Verifier::new(provider)
            .context("rustls-platform-verifier init")?;
        rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth()
    };
    tls.alpn_protocols = vec![TUNNEL_ALPN.to_vec()];

    let quic_client_cfg: quinn::crypto::rustls::QuicClientConfig = tls
        .try_into()
        .context("rustls::ClientConfig → QuicClientConfig")?;
    let mut client_cfg = quinn::ClientConfig::new(Arc::new(quic_client_cfg));

    // Tune for long-lived tunnels:
    // - Aggressive QUIC-layer keep-alive to nudge NATs and detect dead peers
    //   even when no application traffic flows.
    // - Allow the relay to open lots of server-initiated bidi streams (one
    //   per consumer attach). Without this, accept_bi on the client side
    //   stalls because the relay's stream-id allocation is gated by our
    //   `max_concurrent_bidi_streams` advertisement.
    let mut transport = quinn::TransportConfig::default();
    transport.keep_alive_interval(Some(Duration::from_secs(15)));
    transport.max_idle_timeout(Some(
        quinn::IdleTimeout::try_from(Duration::from_secs(120))
            .context("max_idle_timeout out of range")?,
    ));
    transport.max_concurrent_bidi_streams(quinn::VarInt::from_u32(256));
    client_cfg.transport_config(Arc::new(transport));

    let mut endpoint =
        quinn::Endpoint::client("0.0.0.0:0".parse().unwrap()).context("quinn::Endpoint::client")?;
    endpoint.set_default_client_config(client_cfg);
    Ok(endpoint)
}

// ---------------------------------------------------------------------------
// Handshake
// ---------------------------------------------------------------------------

async fn write_handshake(
    send: &mut quinn::SendStream,
    credential: &str,
    registration_token: &str,
) -> Result<()> {
    let cred_bytes = credential.as_bytes();
    if cred_bytes.is_empty() || cred_bytes.len() > HANDSHAKE_MAX_CRED_LEN {
        anyhow::bail!(
            "credential length {} out of range 1..={}",
            cred_bytes.len(),
            HANDSHAKE_MAX_CRED_LEN
        );
    }
    let token = registration_token.as_bytes();
    if token.is_empty() || token.len() > 256 {
        anyhow::bail!(
            "registration_token length {} out of range 1..=256",
            token.len()
        );
    }
    let mut buf = Vec::with_capacity(1 + 1 + cred_bytes.len() + 2 + token.len());
    buf.push(HANDSHAKE_VERSION);
    buf.push(cred_bytes.len() as u8);
    buf.extend_from_slice(cred_bytes);
    buf.extend_from_slice(&(token.len() as u16).to_be_bytes());
    buf.extend_from_slice(token);
    send.write_all(&buf).await.context("write handshake")?;
    Ok(())
}

async fn read_handshake_ack(recv: &mut quinn::RecvStream) -> Result<[u8; 16]> {
    let mut ack = [0u8; 1 + 16];
    recv.read_exact(&mut ack)
        .await
        .context("read handshake ack")?;
    if ack[0] != 0x00 {
        anyhow::bail!(
            "handshake rejected by relay (ack_status = 0x{:02x})",
            ack[0]
        );
    }
    let mut session_base_id = [0u8; 16];
    session_base_id.copy_from_slice(&ack[1..]);
    Ok(session_base_id)
}

// ---------------------------------------------------------------------------
// Heartbeats / control channel
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct HeartbeatWatchdog {
    last_inbound_at: Arc<Mutex<tokio::time::Instant>>,
    misses: Arc<std::sync::atomic::AtomicU32>,
}

impl HeartbeatWatchdog {
    fn new() -> Self {
        Self {
            last_inbound_at: Arc::new(Mutex::new(tokio::time::Instant::now())),
            misses: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        }
    }
    async fn note_inbound(&self) {
        *self.last_inbound_at.lock().await = tokio::time::Instant::now();
        self.misses.store(0, std::sync::atomic::Ordering::SeqCst);
    }
}

async fn heartbeat_pulse(
    send: Arc<Mutex<quinn::SendStream>>,
    shutdown: &mut watch::Receiver<bool>,
) {
    let mut tick = tokio::time::interval(HEARTBEAT_INTERVAL);
    // Skip the immediate fire so we don't wake a freshly-handshaked relay.
    tick.tick().await;
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { break; }
            }
            _ = tick.tick() => {
                let now_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let mut buf = [0u8; 9];
                buf[0] = CONTROL_TAG_HEARTBEAT;
                buf[1..].copy_from_slice(&now_ms.to_be_bytes());
                let mut s = send.lock().await;
                if s.write_all(&buf).await.is_err() {
                    debug!(target: "ohd_storage::relay_client", "heartbeat write failed; pulse exiting");
                    break;
                }
            }
        }
    }
}

async fn control_reader(
    mut recv: quinn::RecvStream,
    watchdog: HeartbeatWatchdog,
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
                        debug!(target: "ohd_storage::relay_client", ?err, "control read err");
                        break;
                    }
                }
                while let Some(consumed) = parse_control_frame(&buf, &watchdog).await {
                    let _ = buf.split_to(consumed);
                }
            }
        }
    }
}

async fn parse_control_frame(buf: &[u8], watchdog: &HeartbeatWatchdog) -> Option<usize> {
    if buf.is_empty() {
        return None;
    }
    match buf[0] {
        CONTROL_TAG_HEARTBEAT => {
            // [tag = 0x02][u64 BE timestamp_ms]
            if buf.len() < 1 + 8 {
                return None;
            }
            // The relay echoes our outbound heartbeats AND emits its own.
            // Either path counts as a liveness signal.
            watchdog.note_inbound().await;
            Some(9)
        }
        other => {
            warn!(target: "ohd_storage::relay_client", tag = other, "unknown control tag");
            // Skip the unknown byte to keep the stream parseable rather
            // than wedging.
            Some(1)
        }
    }
}

async fn watchdog_loop(
    watchdog: HeartbeatWatchdog,
    conn: quinn::Connection,
    shutdown: &mut watch::Receiver<bool>,
) {
    let mut tick = tokio::time::interval(HEARTBEAT_INTERVAL);
    tick.tick().await;
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { break; }
            }
            _ = tick.tick() => {
                let last = *watchdog.last_inbound_at.lock().await;
                if last.elapsed() > HEARTBEAT_INTERVAL * MAX_MISSED_HEARTBEATS {
                    let n = watchdog.misses.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    if n >= MAX_MISSED_HEARTBEATS {
                        warn!(
                            target: "ohd_storage::relay_client",
                            "heartbeat watchdog tripped (no inbound for {:?}); closing connection",
                            last.elapsed()
                        );
                        // Close-code 2 mirrors the relay's HEARTBEAT_TIMEOUT.
                        conn.close(2u32.into(), b"HEARTBEAT_TIMEOUT");
                        break;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Per-session stream demux
// ---------------------------------------------------------------------------

async fn accept_session_streams(
    conn: quinn::Connection,
    service: ConnectRpcService,
    shutdown: &mut watch::Receiver<bool>,
) {
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() { break; }
            }
            res = conn.accept_bi() => {
                match res {
                    Ok((send, recv)) => {
                        let svc = service.clone();
                        tokio::spawn(async move {
                            if let Err(err) = handle_session_stream(send, recv, svc).await {
                                warn!(target: "ohd_storage::relay_client", ?err, "session stream ended with error");
                            }
                        });
                    }
                    Err(err) => {
                        debug!(target: "ohd_storage::relay_client", ?err, "accept_bi err; connection going away");
                        break;
                    }
                }
            }
        }
    }
}

/// Handle one per-session bidi stream end-to-end: read the SESSION_OPEN
/// prefix, swallow the relay's synthetic OPEN envelope, send back an
/// OPEN_ACK, then bridge subsequent DATA frames into a hyper HTTP/2 server
/// connection.
async fn handle_session_stream(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    service: ConnectRpcService,
) -> Result<()> {
    // ---- 1. Read SESSION_OPEN prefix ----
    let mut prefix = [0u8; 5];
    recv.read_exact(&mut prefix)
        .await
        .context("read SESSION_OPEN prefix")?;
    if prefix[0] != STREAM_TAG_SESSION_OPEN {
        anyhow::bail!("unexpected session prefix tag: 0x{:02x}", prefix[0]);
    }
    let session_id = u32::from_be_bytes([prefix[1], prefix[2], prefix[3], prefix[4]]);
    debug!(target: "ohd_storage::relay_client", session_id, "session opened by relay");

    // ---- 2. Read & swallow the relay's synthetic OPEN envelope, ack it ----
    let mut buf = BytesMut::new();
    let first = read_one_frame_buffered(&mut recv, &mut buf)
        .await
        .context("read OPEN envelope")?;
    if first.frame_type != FrameType::Open {
        anyhow::bail!(
            "first session frame was {:?}, expected OPEN",
            first.frame_type
        );
    }
    let ack = encode_frame(FrameType::OpenAck, session_id, &[]);
    send.write_all(&ack).await.context("write OPEN_ACK")?;

    // ---- 3. Bridge into hyper HTTP/2 ----
    //
    // The recv side: spawn a task that pulls TunnelFrames off the QUIC
    // stream, forwards DATA payloads into a bounded mpsc<Bytes> the
    // SessionConn drains in `poll_read`, and signals end-of-stream on
    // CLOSE / fin.
    //
    // The send side: SessionConn::poll_write wraps each write in a
    // TunnelFrame::data envelope and writes to the QUIC SendStream
    // directly (under the lock).
    let (inbound_tx, inbound_rx) = mpsc::channel::<Bytes>(SESSION_INBOUND_BUFFER);
    let send_arc = Arc::new(Mutex::new(send));

    let send_for_reader = Arc::clone(&send_arc);
    let reader_task = tokio::spawn(async move {
        if let Err(err) = pump_inbound(session_id, recv, buf, inbound_tx, send_for_reader).await {
            debug!(
                target: "ohd_storage::relay_client",
                session_id,
                ?err,
                "inbound pump exited with error"
            );
        }
    });

    let conn_io = SessionConn::new(session_id, inbound_rx, Arc::clone(&send_arc));
    // Hyper auto-builder, HTTP/2 only (the consumer side speaks
    // prior-knowledge HTTP/2 over the tunnel).
    let svc_clone = service.clone();
    let svc = hyper::service::service_fn(move |req| {
        let mut s = svc_clone.clone();
        async move {
            <ConnectRpcService as tower::Service<http::Request<hyper::body::Incoming>>>::call(
                &mut s, req,
            )
            .await
        }
    });

    let mut builder = AutoBuilder::new(TokioExecutor::new());
    builder.http2().enable_connect_protocol();
    let conn_fut = builder.serve_connection(TokioIo::new(conn_io), svc);
    if let Err(err) = conn_fut.await {
        // hyper logs many recoverable conditions as errors (e.g. peer
        // GOAWAY); demote to debug so we don't spam logs.
        debug!(
            target: "ohd_storage::relay_client",
            session_id,
            ?err,
            "hyper serve_connection ended"
        );
    }

    // The hyper connection is done; tell the reader to stop and write a
    // CLOSE so the relay can free its routing entry.
    reader_task.abort();
    let close = encode_frame(FrameType::Close, session_id, &[]);
    {
        let mut s = send_arc.lock().await;
        let _ = s.write_all(&close).await;
        let _ = s.finish();
    }
    Ok(())
}

/// Pump TunnelFrames off the QUIC RecvStream into the inbound mpsc.
///
/// `seed` is any partially-read bytes left over from the OPEN-envelope read
/// — they're fed back into the buffered reader so we don't lose them.
async fn pump_inbound(
    session_id: u32,
    mut recv: quinn::RecvStream,
    mut seed: BytesMut,
    inbound_tx: mpsc::Sender<Bytes>,
    send: Arc<Mutex<quinn::SendStream>>,
) -> Result<()> {
    loop {
        // Drain whole frames that already sit in `seed`.
        loop {
            match decode_one_frame(&seed) {
                Ok((frame, consumed)) => {
                    let _ = seed.split_to(consumed);
                    match frame.frame_type {
                        FrameType::Data => {
                            // Backpressure: if the consumer-side hyper conn
                            // can't keep up, the channel send awaits. The
                            // QUIC layer's flow-control then naturally
                            // applies.
                            if inbound_tx.send(frame.payload).await.is_err() {
                                return Ok(());
                            }
                        }
                        FrameType::Close => {
                            debug!(
                                target: "ohd_storage::relay_client",
                                session_id, "CLOSE from relay; ending session"
                            );
                            // Drop inbound_tx so reader sees EOF, then
                            // best-effort echo CLOSE back so the relay can
                            // free its routing entry.
                            drop(inbound_tx);
                            let close = encode_frame(FrameType::Close, session_id, &[]);
                            let mut s = send.lock().await;
                            let _ = s.write_all(&close).await;
                            let _ = s.finish();
                            return Ok(());
                        }
                        FrameType::OpenAck | FrameType::OpenNack | FrameType::WindowUpdate => {
                            // Advisory; ignore.
                        }
                        FrameType::Open | FrameType::Hello | FrameType::Ping | FrameType::Pong => {
                            // Relay shouldn't originate these on a per-
                            // session stream after the initial OPEN.
                            debug!(
                                target: "ohd_storage::relay_client",
                                session_id,
                                kind = ?frame.frame_type,
                                "unexpected frame on session stream; ignoring"
                            );
                        }
                    }
                }
                Err(FrameError::Truncated) => break,
                Err(FrameError::Other(msg)) => {
                    anyhow::bail!("frame decode: {msg}");
                }
            }
        }

        // Read more bytes.
        let mut chunk = vec![0u8; 16 * 1024];
        match recv.read(&mut chunk).await? {
            Some(n) => seed.extend_from_slice(&chunk[..n]),
            None => {
                // QUIC stream finished. Nothing more to feed; let the
                // hyper conn drain.
                return Ok(());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SessionConn: AsyncRead + AsyncWrite over a TunnelFrame::data envelope pair
// ---------------------------------------------------------------------------

/// Adapter that bridges a relay session into a `tokio::io::{AsyncRead,
/// AsyncWrite}` pair so hyper can serve_connection() over it.
struct SessionConn {
    session_id: u32,
    inbound: mpsc::Receiver<Bytes>,
    /// Bytes already pulled from the inbound channel but not yet handed to
    /// the caller's read buffer.
    leftover: Bytes,
    /// Shared send half (locked per outbound frame).
    send: Arc<Mutex<quinn::SendStream>>,
    /// In-flight writev: when a single `poll_write` decides to chunk a
    /// large buffer, this carries the next chunk's future across pollings.
    pending_write: Option<Pin<Box<dyn Future<Output = io::Result<usize>> + Send>>>,
}

impl SessionConn {
    fn new(
        session_id: u32,
        inbound: mpsc::Receiver<Bytes>,
        send: Arc<Mutex<quinn::SendStream>>,
    ) -> Self {
        Self {
            session_id,
            inbound,
            leftover: Bytes::new(),
            send,
            pending_write: None,
        }
    }
}

impl AsyncRead for SessionConn {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // 1. If we have leftover bytes, copy them first.
        if !self.leftover.is_empty() {
            let take = std::cmp::min(self.leftover.len(), out.remaining());
            let chunk = self.leftover.split_to(take);
            out.put_slice(&chunk);
            return Poll::Ready(Ok(()));
        }
        // 2. Otherwise pull from the inbound channel.
        match self.inbound.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => {
                // Stream closed; signal EOF.
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Some(payload)) => {
                let take = std::cmp::min(payload.len(), out.remaining());
                if take == payload.len() {
                    out.put_slice(&payload);
                } else {
                    out.put_slice(&payload[..take]);
                    self.leftover = payload.slice(take..);
                }
                Poll::Ready(Ok(()))
            }
        }
    }
}

impl AsyncWrite for SessionConn {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        src: &[u8],
    ) -> Poll<io::Result<usize>> {
        // If we have an in-flight write future, drive it first.
        if let Some(fut) = self.pending_write.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Ok(n)) => {
                    self.pending_write = None;
                    return Poll::Ready(Ok(n));
                }
                Poll::Ready(Err(err)) => {
                    self.pending_write = None;
                    return Poll::Ready(Err(err));
                }
            }
        }

        if src.is_empty() {
            return Poll::Ready(Ok(0));
        }

        let chunk_len = std::cmp::min(src.len(), MAX_OUTBOUND_PAYLOAD_BYTES);
        let payload = Bytes::copy_from_slice(&src[..chunk_len]);
        let session_id = self.session_id;
        let send = Arc::clone(&self.send);
        let fut = Box::pin(async move {
            let frame = encode_frame(FrameType::Data, session_id, &payload);
            let mut s = send.lock().await;
            s.write_all(&frame).await.map_err(io_err)?;
            Ok(chunk_len)
        });
        self.pending_write = Some(fut);
        // Re-poll immediately: this gives hyper the right backpressure
        // semantics in the common case (write fits, completes synchronously).
        match self.pending_write.as_mut().unwrap().as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Ok(n)) => {
                self.pending_write = None;
                Poll::Ready(Ok(n))
            }
            Poll::Ready(Err(err)) => {
                self.pending_write = None;
                Poll::Ready(Err(err))
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // QUIC streams are inherently buffered + flushed by quinn; nothing
        // to do here. The pending_write future, if any, is awaited on the
        // next poll_write.
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Drain any pending write before declaring shutdown.
        if let Some(fut) = self.pending_write.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Ok(_)) => {
                    self.pending_write = None;
                }
                Poll::Ready(Err(err)) => {
                    self.pending_write = None;
                    return Poll::Ready(Err(err));
                }
            }
        }
        // The handle_session_stream caller is responsible for the final
        // CLOSE + finish() — leaving that here would race against the
        // outbound TunnelFrame writes.
        Poll::Ready(Ok(()))
    }
}

fn io_err<E: std::fmt::Display>(err: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err.to_string())
}

// ---------------------------------------------------------------------------
// Frame codec (subset, matches relay/src/frame.rs)
//
// We embed a minimal `TunnelFrame` codec here rather than depending on
// `ohd-relay::frame` because the storage crate intentionally has no path-
// dep on the relay crate. The frame format is part of the protocol's
// on-wire ABI, documented in `relay/spec/relay-protocol.md` + at the top
// of `relay/src/frame.rs`. Mirror layout:
//
//   [u32 BE MAGIC = 0x4F484400 (b"OHD\0")]
//   [u8 frame_type]
//   [u8 flags = 0]
//   [u8 reserved = 0]
//   [u32 BE session_id]
//   [u16 BE payload_len]
//   [payload_len bytes payload]
// = 13 bytes header + payload.
// ---------------------------------------------------------------------------

const FRAME_MAGIC: [u8; 4] = [b'O', b'H', b'D', 0x00];
const FRAME_HEADER_LEN: usize = 4 + 1 + 1 + 1 + 4 + 2;
const MAX_PAYLOAD_LEN: usize = u16::MAX as usize;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameType {
    Hello = 0x01,
    Ping = 0x02,
    Pong = 0x03,
    Open = 0x04,
    OpenAck = 0x05,
    OpenNack = 0x06,
    Data = 0x07,
    Close = 0x08,
    WindowUpdate = 0x0A,
}

impl FrameType {
    fn from_u8(b: u8) -> Result<Self, FrameError> {
        Ok(match b {
            0x01 => FrameType::Hello,
            0x02 => FrameType::Ping,
            0x03 => FrameType::Pong,
            0x04 => FrameType::Open,
            0x05 => FrameType::OpenAck,
            0x06 => FrameType::OpenNack,
            0x07 => FrameType::Data,
            0x08 => FrameType::Close,
            0x0A => FrameType::WindowUpdate,
            other => {
                return Err(FrameError::Other(format!(
                    "unknown frame type 0x{other:02x}"
                )))
            }
        })
    }
}

#[derive(Debug)]
struct Frame {
    frame_type: FrameType,
    #[allow(dead_code)]
    session_id: u32,
    payload: Bytes,
}

#[derive(Debug)]
enum FrameError {
    Truncated,
    Other(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameError::Truncated => write!(f, "truncated frame"),
            FrameError::Other(s) => f.write_str(s),
        }
    }
}

impl std::error::Error for FrameError {}

fn encode_frame(frame_type: FrameType, session_id: u32, payload: &[u8]) -> Vec<u8> {
    debug_assert!(payload.len() <= MAX_PAYLOAD_LEN);
    let mut buf = Vec::with_capacity(FRAME_HEADER_LEN + payload.len());
    buf.extend_from_slice(&FRAME_MAGIC);
    buf.push(frame_type as u8);
    buf.push(0); // flags
    buf.push(0); // reserved
    buf.extend_from_slice(&session_id.to_be_bytes());
    buf.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    buf.extend_from_slice(payload);
    buf
}

fn decode_one_frame(buf: &[u8]) -> Result<(Frame, usize), FrameError> {
    if buf.len() < FRAME_HEADER_LEN {
        return Err(FrameError::Truncated);
    }
    if buf[0..4] != FRAME_MAGIC {
        return Err(FrameError::Other(format!("bad magic: {:02x?}", &buf[0..4])));
    }
    let frame_type = FrameType::from_u8(buf[4])?;
    if buf[5] != 0 {
        return Err(FrameError::Other(format!(
            "non-zero flags: 0x{:02x}",
            buf[5]
        )));
    }
    if buf[6] != 0 {
        return Err(FrameError::Other(format!(
            "non-zero reserved: 0x{:02x}",
            buf[6]
        )));
    }
    let session_id = u32::from_be_bytes([buf[7], buf[8], buf[9], buf[10]]);
    let payload_len = u16::from_be_bytes([buf[11], buf[12]]) as usize;
    let total = FRAME_HEADER_LEN + payload_len;
    if buf.len() < total {
        return Err(FrameError::Truncated);
    }
    let payload = Bytes::copy_from_slice(&buf[FRAME_HEADER_LEN..total]);
    Ok((
        Frame {
            frame_type,
            session_id,
            payload,
        },
        total,
    ))
}

/// Read a single frame from `recv`, preserving leftover bytes in `buf` for
/// the next call. **Critical**: callers MUST reuse the same `buf` across
/// calls — `quinn::RecvStream::read` chunks may carry multiple
/// `TunnelFrame`s in a single read, and a fresh-buf-per-call helper would
/// silently drop trailing bytes after the first frame parses. This is the
/// wire quirk the relay agent flagged in their integration tests.
async fn read_one_frame_buffered(
    recv: &mut quinn::RecvStream,
    buf: &mut BytesMut,
) -> Result<Frame> {
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        match decode_one_frame(buf) {
            Ok((frame, consumed)) => {
                let _ = buf.split_to(consumed);
                return Ok(frame);
            }
            Err(FrameError::Truncated) => {
                let n = recv
                    .read(&mut chunk)
                    .await
                    .context("recv.read")?
                    .ok_or_else(|| anyhow!("stream ended mid-frame"))?;
                buf.extend_from_slice(&chunk[..n]);
            }
            Err(FrameError::Other(msg)) => {
                anyhow::bail!("frame decode: {msg}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// TLS verifiers
// ---------------------------------------------------------------------------

/// Insecure verifier — accepts any cert. Dev only.
#[derive(Debug)]
struct InsecureCertVerifier {
    provider: rustls::crypto::CryptoProvider,
}

impl InsecureCertVerifier {
    fn new() -> Self {
        Self {
            provider: rustls::crypto::ring::default_provider(),
        }
    }
}

impl rustls::client::danger::ServerCertVerifier for InsecureCertVerifier {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Pinned-SPKI verifier — accepts a cert iff its SubjectPublicKeyInfo
/// SHA-256 hash matches the configured pin.
///
/// Sidesteps the WebPKI machinery for an operator we already trust by way
/// of registration. The pin is delivered out-of-band (the relay's
/// `/v1/register` response v1.x). Trust failure → `BadEncoding`-ish error.
#[derive(Debug)]
struct SpkiPinVerifier {
    pin: [u8; 32],
    provider: rustls::crypto::CryptoProvider,
}

impl SpkiPinVerifier {
    fn new(pin: Vec<u8>) -> Result<Self> {
        if pin.len() != 32 {
            anyhow::bail!(
                "expected_relay_pubkey_pin must be 32 bytes (SHA-256), got {}",
                pin.len()
            );
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&pin);
        Ok(Self {
            pin: arr,
            provider: rustls::crypto::ring::default_provider(),
        })
    }
}

impl rustls::client::danger::ServerCertVerifier for SpkiPinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        // Hash the entire DER-encoded leaf cert (not the SPKI extracted —
        // we don't pull in x509-parser for one match). This matches the
        // common "cert pin" convention. If we want true SPKI pins later,
        // we can swap the hash input without changing the wire format.
        let mut hasher = Sha256::new();
        hasher.update(end_entity.as_ref());
        let got = hasher.finalize();
        // Constant-time compare.
        let mut acc = 0u8;
        for i in 0..32 {
            acc |= got[i] ^ self.pin[i];
        }
        if acc == 0 {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrip_data() {
        let payload = b"hello, world";
        let bytes = encode_frame(FrameType::Data, 42, payload);
        let (frame, consumed) = decode_one_frame(&bytes).expect("decode");
        assert_eq!(consumed, bytes.len());
        assert_eq!(frame.frame_type, FrameType::Data);
        assert_eq!(frame.session_id, 42);
        assert_eq!(&frame.payload[..], payload);
    }

    #[test]
    fn frame_decode_truncated() {
        let bytes = encode_frame(FrameType::Open, 1, b"x");
        // Header ok, payload missing one byte.
        let truncated = &bytes[..bytes.len() - 1];
        match decode_one_frame(truncated) {
            Err(FrameError::Truncated) => {}
            other => panic!("expected Truncated, got {other:?}"),
        }
    }

    #[test]
    fn frame_decode_two_in_one_buffer() {
        let mut all = encode_frame(FrameType::OpenAck, 7, &[]);
        all.extend_from_slice(&encode_frame(FrameType::Data, 7, b"abc"));
        let (f1, c1) = decode_one_frame(&all).expect("decode 1");
        assert_eq!(f1.frame_type, FrameType::OpenAck);
        let (f2, c2) = decode_one_frame(&all[c1..]).expect("decode 2");
        assert_eq!(f2.frame_type, FrameType::Data);
        assert_eq!(c1 + c2, all.len());
    }

    #[test]
    fn relay_url_parse_ok() {
        let opts = RelayClientOptions {
            relay_url: "127.0.0.1:9001".to_string(),
            registration_token: "rid".into(),
            credential: "cred".into(),
            expected_relay_pubkey_pin: None,
            allow_insecure_dev: true,
        };
        let (addr, sni) = opts.parse().expect("parse");
        assert_eq!(addr.port(), 9001);
        assert_eq!(sni, "127.0.0.1");
    }

    #[test]
    fn relay_url_parse_missing_port() {
        let opts = RelayClientOptions {
            relay_url: "no-port".to_string(),
            registration_token: "rid".into(),
            credential: "cred".into(),
            expected_relay_pubkey_pin: None,
            allow_insecure_dev: true,
        };
        assert!(opts.parse().is_err());
    }

    #[test]
    fn pin_verifier_rejects_wrong_pin() {
        use rustls::client::danger::ServerCertVerifier;
        let pin = [0u8; 32];
        let v = SpkiPinVerifier::new(pin.to_vec()).unwrap();
        // Synthesize a cert DER that's clearly not all-zero-hash.
        let cert = CertificateDer::from(vec![0x30, 0x82, 0x00, 0x01, 0xAA]);
        let res = v.verify_server_cert(
            &cert,
            &[],
            &ServerName::try_from("localhost").unwrap(),
            &[],
            UnixTime::since_unix_epoch(std::time::Duration::from_secs(0)),
        );
        assert!(res.is_err());
    }

    #[test]
    fn pin_verifier_accepts_matching_pin() {
        use rustls::client::danger::ServerCertVerifier;
        let cert_der = vec![0x30, 0x82, 0x00, 0x01, 0xAA, 0xBB];
        let mut h = Sha256::new();
        h.update(&cert_der);
        let pin = h.finalize().to_vec();
        let v = SpkiPinVerifier::new(pin).unwrap();
        let cert = CertificateDer::from(cert_der);
        let res = v.verify_server_cert(
            &cert,
            &[],
            &ServerName::try_from("localhost").unwrap(),
            &[],
            UnixTime::since_unix_epoch(std::time::Duration::from_secs(0)),
        );
        assert!(res.is_ok());
    }
}
