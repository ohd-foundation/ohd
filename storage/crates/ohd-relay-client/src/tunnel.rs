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
//! mirror.
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
//!   - Then opaque [`crate::frame`] envelopes flow both ways.
//! - Heartbeats: app-level on the control channel,
//!   `[u8 = HEARTBEAT (0x02)][u64 BE timestamp_ms]`.
//!
//! # Wire quirk: buffered reader
//!
//! `quinn::RecvStream::read_chunk` may carry multiple frames in a single
//! read. A fresh-buf-per-call reader silently loses bytes after the first
//! frame parses. [`read_one_frame_buffered`] preserves leftover bytes
//! across reads. This is critical and easy to get wrong.
//!
//! # Session handling
//!
//! Each accepted per-session stream pair is handed to a [`SessionHandler`].
//! The crate ships one implementation behind the `tunnel-service` feature:
//! [`crate::service::ConnectRpcSessionHandler`], which bridges the session
//! into a hyper HTTP/2 server connection running a
//! `connectrpc::ConnectRpcService`. That handler pulls heavy server-only
//! deps (hyper, connectrpc, tower) — hence the feature gate. The Android
//! uniffi binding builds without it and supplies its own handler.

use std::future::Future;
use std::net::{SocketAddr, ToSocketAddrs};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context as _, Result};
use bytes::BytesMut;
use tokio::sync::{watch, Mutex};
use tracing::{debug, info, warn};

use crate::frame::{decode_one_frame, Frame, FrameError};

// ---------------------------------------------------------------------------
// Wire constants — must match `ohd-relay::quic_tunnel` exactly.
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
///
/// Shortened from 60 s to 20 s. This also drives the watchdog window below
/// (`HEARTBEAT_INTERVAL * MAX_MISSED_HEARTBEATS`), so a genuinely dead tunnel
/// is detected — and reconnect kicks in — in ~120 s instead of ~6 min.
///
/// NOTE: this does NOT fully resolve the observed tunnel-stability bug. A
/// packet capture showed phone→relay traffic stops ~18 s after the handshake
/// (neither quinn's 15 s transport keep-alive nor this heartbeat reaches the
/// relay after that point), so the relay idle-times-out the QUIC connection
/// at ~120 s. Root cause is still under investigation — see the tunnel
/// notes; a two-ended packet capture is the next diagnostic.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);

/// Maximum consecutive missed heartbeats before tearing down the connection.
pub const MAX_MISSED_HEARTBEATS: u32 = 3;

/// Initial reconnect delay; doubles per failure up to [`RECONNECT_DELAY_CAP`].
pub const RECONNECT_DELAY_INITIAL: Duration = Duration::from_secs(1);

/// Cap on reconnect backoff.
pub const RECONNECT_DELAY_CAP: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Configuration for the storage-side relay tunnel client.
#[derive(Debug, Clone)]
pub struct RelayClientOptions {
    /// `host:port` of the relay's `--quic-tunnel-listen` endpoint, e.g.
    /// `relay.example.org:9001`. The host portion is the TLS SNI.
    pub relay_url: String,
    /// The `rendezvous_id` issued at registration time.
    pub registration_token: String,
    /// The `long_lived_credential` from registration.
    pub credential: String,
    /// Optional SHA-256 pin of the relay's QUIC tunnel cert.
    pub expected_relay_pubkey_pin: Option<Vec<u8>>,
    /// Dev only: accept any server cert without verification.
    pub allow_insecure_dev: bool,
}

impl RelayClientOptions {
    /// Resolve `relay_url` to a [`SocketAddr`] + an SNI/host string.
    pub(crate) fn parse(&self) -> Result<(SocketAddr, String)> {
        let (host, port) = self
            .relay_url
            .rsplit_once(':')
            .ok_or_else(|| anyhow!("relay_url missing :port suffix: {}", self.relay_url))?;
        let port: u16 = port
            .parse()
            .with_context(|| format!("relay_url port not a number: {}", self.relay_url))?;
        let addr = (host, port)
            .to_socket_addrs()
            .with_context(|| format!("resolve {}", self.relay_url))?
            .next()
            .ok_or_else(|| anyhow!("no addresses for {}", self.relay_url))?;
        Ok((addr, host.to_string()))
    }
}

/// A per-session stream accepted off the relay tunnel.
///
/// The [`SessionHandler`] receives the SESSION_OPEN-prefixed session id, the
/// raw `quinn` send/recv halves, and the OPEN-envelope frame the relay
/// pushed first (already decoded and verified to be of type `Open`).
pub struct AcceptedSession {
    /// Relay-assigned session id from the SESSION_OPEN prefix.
    pub session_id: u32,
    /// QUIC send half for this session stream.
    pub send: quinn::SendStream,
    /// QUIC recv half for this session stream.
    pub recv: quinn::RecvStream,
    /// The relay's synthetic OPEN envelope (already consumed off `recv`).
    pub open_frame: Frame,
    /// Any bytes read past the OPEN envelope — feed back into the reader so
    /// they are not lost (see the buffered-reader wire quirk).
    pub leftover: BytesMut,
}

/// Handles each per-session stream accepted off the tunnel.
///
/// Implementors bridge the opaque session bytes into whatever they like —
/// the server crate bridges into a local `ConnectRpcService` over hyper
/// HTTP/2; the Android binding wires its own.
pub trait SessionHandler: Send + Sync + 'static {
    /// Drive one session to completion. Errors are logged by the caller.
    fn handle(
        &self,
        session: AcceptedSession,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send>>;
}

/// Run the outbound tunnel client until `shutdown` flips to `true` or the
/// sender is dropped.
///
/// On every reconnect we re-run the handshake. Each accepted per-session
/// stream is dispatched to `handler`. Errors are logged at `warn`; the loop
/// never exits on transport failure (only on shutdown).
pub async fn serve_relay_tunnel(
    opts: RelayClientOptions,
    handler: Arc<dyn SessionHandler>,
    mut shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let mut delay = RECONNECT_DELAY_INITIAL;
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        match run_one_connection(&opts, Arc::clone(&handler), shutdown.clone()).await {
            Ok(()) => {
                info!(target: "ohd_relay_client::tunnel", "tunnel session ended cleanly; reconnecting");
                delay = RECONNECT_DELAY_INITIAL;
            }
            Err(err) => {
                warn!(target: "ohd_relay_client::tunnel", ?err, ?delay, "tunnel session ended with error; reconnecting after backoff");
            }
        }

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

/// Drive one full QUIC tunnel connection lifecycle: connect, handshake, run
/// the control + accept loops until either side hangs up.
async fn run_one_connection(
    opts: &RelayClientOptions,
    handler: Arc<dyn SessionHandler>,
    shutdown: watch::Receiver<bool>,
) -> Result<()> {
    let (addr, sni) = opts.parse()?;
    let endpoint = build_client_endpoint(opts)?;
    info!(target: "ohd_relay_client::tunnel", %addr, %sni, "dialing relay tunnel");

    let conn = endpoint
        .connect(addr, &sni)
        .context("quinn::Endpoint::connect")?
        .await
        .context("QUIC handshake")?;

    let (mut ctrl_send, mut ctrl_recv) = conn.open_bi().await.context("open_bi (handshake)")?;

    write_handshake(&mut ctrl_send, &opts.credential, &opts.registration_token).await?;
    let session_base_id = read_handshake_ack(&mut ctrl_recv).await?;
    info!(
        target: "ohd_relay_client::tunnel",
        session_base_id = %hex::encode(session_base_id),
        "tunnel handshake accepted"
    );

    let ctrl_send = Arc::new(Mutex::new(ctrl_send));

    let conn_for_accept = conn.clone();
    let mut accept_shutdown = shutdown.clone();
    let accept_task = tokio::spawn(async move {
        accept_session_streams(conn_for_accept, handler, &mut accept_shutdown).await;
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
            debug!(target: "ohd_relay_client::tunnel", "connection closed by relay or transport");
        }
        _ = shutdown_wait.changed() => {
            if *shutdown_wait.borrow() {
                debug!(target: "ohd_relay_client::tunnel", "shutdown signaled; deregistering and closing");
                conn.close(0u32.into(), b"shutdown");
            }
        }
        _ = reader_task => {
            debug!(target: "ohd_relay_client::tunnel", "control reader exited");
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
    let tls = crate::tls::build_client_tls_config(
        opts.allow_insecure_dev,
        opts.expected_relay_pubkey_pin.clone(),
        TUNNEL_ALPN,
    )?;

    let quic_client_cfg: quinn::crypto::rustls::QuicClientConfig = tls
        .try_into()
        .context("rustls::ClientConfig → QuicClientConfig")?;
    let mut client_cfg = quinn::ClientConfig::new(Arc::new(quic_client_cfg));

    // Tune for long-lived tunnels: aggressive keep-alive + allow the relay
    // to open many server-initiated bidi streams.
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
                    debug!(target: "ohd_relay_client::tunnel", "heartbeat write failed; pulse exiting");
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
                        debug!(target: "ohd_relay_client::tunnel", ?err, "control read err");
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
            if buf.len() < 1 + 8 {
                return None;
            }
            watchdog.note_inbound().await;
            Some(9)
        }
        other => {
            warn!(target: "ohd_relay_client::tunnel", tag = other, "unknown control tag");
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
                            target: "ohd_relay_client::tunnel",
                            "heartbeat watchdog tripped (no inbound for {:?}); closing connection",
                            last.elapsed()
                        );
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
    handler: Arc<dyn SessionHandler>,
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
                        let h = Arc::clone(&handler);
                        tokio::spawn(async move {
                            if let Err(err) = dispatch_session_stream(send, recv, h).await {
                                warn!(target: "ohd_relay_client::tunnel", ?err, "session stream ended with error");
                            }
                        });
                    }
                    Err(err) => {
                        debug!(target: "ohd_relay_client::tunnel", ?err, "accept_bi err; connection going away");
                        break;
                    }
                }
            }
        }
    }
}

/// Read the SESSION_OPEN prefix + the relay's synthetic OPEN envelope, then
/// hand the session off to the [`SessionHandler`].
async fn dispatch_session_stream(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    handler: Arc<dyn SessionHandler>,
) -> Result<()> {
    let mut prefix = [0u8; 5];
    recv.read_exact(&mut prefix)
        .await
        .context("read SESSION_OPEN prefix")?;
    if prefix[0] != STREAM_TAG_SESSION_OPEN {
        anyhow::bail!("unexpected session prefix tag: 0x{:02x}", prefix[0]);
    }
    let session_id = u32::from_be_bytes([prefix[1], prefix[2], prefix[3], prefix[4]]);
    debug!(target: "ohd_relay_client::tunnel", session_id, "session opened by relay");

    let mut leftover = BytesMut::new();
    let open_frame = read_one_frame_buffered(&mut recv, &mut leftover)
        .await
        .context("read OPEN envelope")?;
    if open_frame.frame_type != crate::frame::FrameType::Open {
        anyhow::bail!(
            "first session frame was {:?}, expected OPEN",
            open_frame.frame_type
        );
    }
    // Swallow nothing else here — the OPEN_ACK and DATA bridging are the
    // handler's job (the server handler writes OPEN_ACK before bridging).
    let _ = &mut send;
    handler
        .handle(AcceptedSession {
            session_id,
            send,
            recv,
            open_frame,
            leftover,
        })
        .await
}

// ---------------------------------------------------------------------------
// Buffered frame reader
// ---------------------------------------------------------------------------

/// Read a single frame from `recv`, preserving leftover bytes in `buf` for
/// the next call. **Critical**: callers MUST reuse the same `buf` across
/// calls — `quinn::RecvStream::read` chunks may carry multiple frames in a
/// single read, and a fresh-buf-per-call helper would silently drop
/// trailing bytes after the first frame parses.
pub async fn read_one_frame_buffered(
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{encode_frame, FrameType};

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
    fn handshake_frame_layout() {
        // The handshake buffer the relay parses:
        //   [v][cred_len][cred][token_len BE][token]
        let cred = "credxyz";
        let token = "rendezvous-token";
        let mut buf = Vec::new();
        buf.push(HANDSHAKE_VERSION);
        buf.push(cred.len() as u8);
        buf.extend_from_slice(cred.as_bytes());
        buf.extend_from_slice(&(token.len() as u16).to_be_bytes());
        buf.extend_from_slice(token.as_bytes());
        assert_eq!(buf[0], 0x01);
        assert_eq!(buf[1] as usize, cred.len());
        let tl = u16::from_be_bytes([buf[2 + cred.len()], buf[2 + cred.len() + 1]]);
        assert_eq!(tl as usize, token.len());
    }

    #[tokio::test]
    async fn control_frame_parser_consumes_heartbeat() {
        let wd = HeartbeatWatchdog::new();
        let mut buf = vec![CONTROL_TAG_HEARTBEAT];
        buf.extend_from_slice(&123u64.to_be_bytes());
        let consumed = parse_control_frame(&buf, &wd).await;
        assert_eq!(consumed, Some(9));
    }

    #[tokio::test]
    async fn control_frame_parser_truncated_heartbeat() {
        let wd = HeartbeatWatchdog::new();
        let buf = vec![CONTROL_TAG_HEARTBEAT, 0, 0]; // < 9 bytes
        assert_eq!(parse_control_frame(&buf, &wd).await, None);
    }

    #[tokio::test]
    async fn control_frame_parser_skips_unknown_tag() {
        let wd = HeartbeatWatchdog::new();
        let buf = vec![0xFF, 0x00];
        assert_eq!(parse_control_frame(&buf, &wd).await, Some(1));
    }

    #[test]
    fn open_envelope_is_an_open_frame() {
        // Sanity: the OPEN envelope dispatch_session_stream expects.
        let env = encode_frame(FrameType::Open, 9, b"ohdg_preview");
        let (frame, consumed) = decode_one_frame(&env).unwrap();
        assert_eq!(frame.frame_type, FrameType::Open);
        assert_eq!(consumed, env.len());
    }
}
