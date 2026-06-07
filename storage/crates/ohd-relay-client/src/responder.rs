//! Phone-side **share responder** — the genuinely new capability of CORD
//! Phase 4d.
//!
//! # Where this fits
//!
//! `cord/spec/data-link.md` "The phone-side share responder": OHD Connect
//! on the phone embeds `ohd-storage-core` + `ohd-mcp-core` and, until now,
//! ran CORD's tool loop *locally, as the owner* — full unscoped access. It
//! could not **serve** a remote consumer (CORD server-side, a clinician's
//! device) and could not enforce a grant's scope on tool output.
//!
//! This module is both. For a single share it:
//!
//! 1. Registers a per-share rendezvous on the relay
//!    ([`register_share_rendezvous`]) — `POST /v1/register`, yielding a
//!    `rendezvous_id` + `long_lived_credential`, and derives the SPKI pin
//!    from the storage identity cert.
//! 2. Maintains the relay tunnel ([`ShareResponder::serve`]) via
//!    [`crate::tunnel::serve_relay_tunnel`] — `OpenTunnel`, heartbeats,
//!    reconnect-with-backoff.
//! 3. For each inbound tunnel session, terminates the **inner TLS** server
//!    side with the storage's self-signed identity cert (`ohd-h3-helpers`'s
//!    `tls_pin`; the consumer pins it), then over that TLS 1.3 session
//!    speaks **MCP** JSON-RPC (`initialize`, `tools/list`, `tools/call`)
//!    backed by [`ohd_mcp_core::catalog_scoped`] / [`dispatch_scoped`] with
//!    the share's [`ShareScope`].
//!
//! The phone is the **enforcement boundary**: the responder never calls
//! `ohd-mcp-core` as the owner — every catalog + dispatch call carries the
//! share's `ShareScope`, derived from the grant's read/write/channel rules
//! and time window. A buggy or compromised CORD cannot exceed the grant.
//!
//! # Inner-TLS wire shape
//!
//! Per `relay/spec/relay-protocol.md` "Inner-TLS wire shape": the inner TLS
//! session rides the ordinary `DATA` frames of an already-opened tunnel
//! session. The relay assigns a `SESSION_ID`, sends `OPEN`, the responder
//! replies `OPEN_ACK`; only then does either side emit a TLS byte. Every
//! TLS record is the payload of a `DATA` frame; the receiver concatenates
//! a session's `DATA` payloads into one byte stream and feeds its TLS
//! engine. [`SessionStream`] is exactly that `AsyncRead + AsyncWrite`
//! bridge.
//!
//! # MCP transport
//!
//! Over the inner-TLS byte stream the responder speaks **newline-delimited
//! JSON-RPC 2.0** — the MCP stdio framing (one JSON object per line). This
//! is the natural fit for a raw TLS stream and keeps the responder free of
//! an embedded HTTP server. The ALPN `ohd-mcp1` ([`tls_pin::INNER_TLS_ALPN`])
//! is negotiated end-to-end and is invisible to the relay.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::{Context as _, Result};
use bytes::{Bytes, BytesMut};
use ohd_h3_helpers::tls_pin::{self, storage_identity_cert};
use ohd_mcp_core::ShareScope;
use ohd_storage_core::Storage;
use serde_json::{json, Value};
use tokio::io::{
    AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, ReadBuf,
};
use tokio::sync::{mpsc, watch, Mutex};
use tracing::{debug, info, warn};

use crate::frame::{decode_one_frame, encode_frame, FrameError, FrameType};
use crate::registration::{RegisterRequest, RegistrationClient, RegistrationError};
use crate::tunnel::{AcceptedSession, RelayClientOptions, SessionHandler};

/// MCP protocol revision the responder advertises in `initialize`.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Per-session inbound channel depth (DATA frames buffered before
/// backpressure).
const SESSION_INBOUND_BUFFER: usize = 256;

/// Cap on a single outbound `DATA` payload — the wire's u16 `payload_len`
/// field with headroom. Larger TLS records are chunked transparently.
const MAX_OUTBOUND_PAYLOAD_BYTES: usize = 60 * 1024;

/// A line longer than this on the MCP stream is rejected — a guard against
/// an unbounded-allocation DoS from a malformed/hostile consumer.
const MAX_MCP_LINE_BYTES: usize = 4 * 1024 * 1024;

// ===========================================================================
// Rendezvous registration
// ===========================================================================

/// Outcome of registering a per-share rendezvous on the relay.
///
/// Each share gets its **own** rendezvous (`cord/spec/data-link.md`
/// "Activating remote access" step 2): revoking one share's remote access
/// never disturbs another.
#[derive(Debug, Clone)]
pub struct ShareRendezvous {
    /// The opaque per-share rendezvous id the relay issued.
    pub rendezvous_id: String,
    /// The relay's public URL for this rendezvous.
    pub rendezvous_url: String,
    /// The `long_lived_credential` authenticating subsequent tunnel opens.
    pub long_lived_credential: String,
    /// `SHA-256(SubjectPublicKeyInfo)` of the storage identity cert,
    /// base64url-no-pad — the `pin=` parameter of the share artifact.
    pub spki_pin_b64url: String,
}

/// Register a per-share rendezvous with the relay and derive the cert pin.
///
/// `relay_origin` is the relay's HTTP origin (e.g. `https://relay.ohd.dev`).
/// `identity_key_pkcs8_der` is the storage's long-lived Ed25519 identity key
/// in PKCS#8 DER form; the same bytes mint the responder's TLS cert in
/// [`ShareResponder`], so the pin returned here is the pin a consumer will
/// see on the wire.
///
/// This is the network half of "Connect registers a per-share rendezvous".
/// It is real — it performs the actual `POST /v1/register`.
pub async fn register_share_rendezvous(
    relay_origin: &str,
    user_ulid_hex: &str,
    identity_key_pkcs8_der: &[u8],
    user_label: Option<String>,
) -> Result<ShareRendezvous, RegistrationError> {
    let client = RegistrationClient::new(relay_origin)?;

    // The SPKI is exactly the Ed25519 identity public key's DER encoding —
    // load the key into rcgen and read it back. The pin is SHA-256 of that
    // SPKI, base64url; it is invariant across the rendezvous URL and cert
    // renewals, so it equals the pin a consumer will see on the wire.
    let key_pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
        &rustls::pki_types::PrivatePkcs8KeyDer::from(identity_key_pkcs8_der.to_vec()),
        &rcgen::PKCS_ED25519,
    )
    .map_err(|e| RegistrationError::BadUrl(format!("load identity key: {e}")))?;
    let spki_der = key_pair.public_key_der();
    let spki_pin_b64url = b64url_no_pad(&sha256(&spki_der));
    let spki_hex = hex::encode(&spki_der);

    let resp = client
        .register(&RegisterRequest {
            user_ulid: user_ulid_hex.to_string(),
            storage_pubkey_spki_hex: spki_hex,
            push_token: None,
            user_label,
            id_token: None,
        })
        .await?;

    Ok(ShareRendezvous {
        rendezvous_id: resp.rendezvous_id,
        rendezvous_url: resp.rendezvous_url,
        long_lived_credential: resp.long_lived_credential,
        spki_pin_b64url,
    })
}

// ===========================================================================
// The share responder
// ===========================================================================

/// Serves one share to remote consumers over the relay tunnel.
///
/// Construct with [`ShareResponder::new`], then drive [`ShareResponder::serve`]
/// on a task; it runs until the supplied shutdown signal flips. While it
/// runs the tunnel is kept open (re-established on transport failure) and
/// every inbound session is answered with scoped MCP.
pub struct ShareResponder {
    storage: Arc<Storage>,
    /// The grant id backing this share — its scope is resolved fresh per
    /// session so a mid-life suspend/revoke takes effect immediately.
    grant_id: i64,
    /// The storage identity key (PKCS#8 DER) — mints the inner-TLS cert.
    identity_key_pkcs8_der: Vec<u8>,
    /// The rendezvous URL, used as the cert SAN.
    rendezvous_url: String,
    /// Relay client options for the QUIC tunnel.
    tunnel_opts: RelayClientOptions,
}

impl ShareResponder {
    /// Build a responder for one share.
    ///
    /// - `storage` — the on-device storage core (shared, thread-safe).
    /// - `grant_id` — the grant backing the share; its [`ShareScope`] is
    ///   resolved per session.
    /// - `identity_key_pkcs8_der` — the storage's Ed25519 identity key.
    /// - `rendezvous` — the registration outcome from
    ///   [`register_share_rendezvous`].
    /// - `relay_tunnel_url` — `host:port` of the relay's QUIC tunnel
    ///   endpoint (the `--quic-tunnel-listen` address).
    /// - `allow_insecure_dev` — accept any relay QUIC cert (tests only).
    pub fn new(
        storage: Arc<Storage>,
        grant_id: i64,
        identity_key_pkcs8_der: Vec<u8>,
        rendezvous: &ShareRendezvous,
        relay_tunnel_url: String,
        allow_insecure_dev: bool,
    ) -> Self {
        let tunnel_opts = RelayClientOptions {
            relay_url: relay_tunnel_url,
            registration_token: rendezvous.rendezvous_id.clone(),
            credential: rendezvous.long_lived_credential.clone(),
            expected_relay_pubkey_pin: None,
            allow_insecure_dev,
        };
        Self {
            storage,
            grant_id,
            identity_key_pkcs8_der,
            rendezvous_url: rendezvous.rendezvous_url.clone(),
            tunnel_opts,
        }
    }

    /// Run the responder until `shutdown` flips to `true` (or its sender is
    /// dropped). Maintains the relay tunnel and serves every session.
    pub async fn serve(self, shutdown: watch::Receiver<bool>) -> Result<()> {
        let tls_acceptor = build_inner_tls_acceptor(
            &self.identity_key_pkcs8_der,
            &self.rendezvous_url,
        )
        .context("build inner-TLS acceptor")?;

        let handler: Arc<dyn SessionHandler> = Arc::new(ShareSessionHandler {
            storage: Arc::clone(&self.storage),
            grant_id: self.grant_id,
            tls_acceptor,
        });

        info!(
            target: "ohd_relay_client::responder",
            grant_id = self.grant_id,
            "share responder serving relay tunnel"
        );
        crate::tunnel::serve_relay_tunnel(self.tunnel_opts, handler, shutdown).await
    }
}

/// Build the `tokio_rustls` TLS 1.3 server acceptor for the inner session.
///
/// The cert is the storage's self-signed identity cert (`tls_pin`); the
/// consumer pins it. ALPN is fixed to `ohd-mcp1` — MCP over the tunnel.
fn build_inner_tls_acceptor(
    identity_key_pkcs8_der: &[u8],
    rendezvous_url: &str,
) -> Result<tokio_rustls::TlsAcceptor> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let ident = storage_identity_cert(identity_key_pkcs8_der, rendezvous_url, now_secs())
        .context("mint storage identity cert")?;
    let mut config = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_no_client_auth()
        .with_single_cert(ident.cert_chain, ident.key)
        .context("inner-TLS ServerConfig")?;
    config.alpn_protocols = vec![tls_pin::INNER_TLS_ALPN.to_vec()];
    Ok(tokio_rustls::TlsAcceptor::from(Arc::new(config)))
}

// ===========================================================================
// Per-session handler: OPEN_ACK → inner TLS → scoped MCP
// ===========================================================================

/// [`SessionHandler`] that terminates inner TLS and speaks scoped MCP.
struct ShareSessionHandler {
    storage: Arc<Storage>,
    grant_id: i64,
    tls_acceptor: tokio_rustls::TlsAcceptor,
}

impl SessionHandler for ShareSessionHandler {
    fn handle(
        &self,
        session: AcceptedSession,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        let storage = Arc::clone(&self.storage);
        let grant_id = self.grant_id;
        let acceptor = self.tls_acceptor.clone();
        Box::pin(async move { serve_session(session, storage, grant_id, acceptor).await })
    }
}

/// Drive one accepted session: ACK the OPEN, terminate inner TLS, run the
/// MCP loop, tear the session down.
async fn serve_session(
    session: AcceptedSession,
    storage: Arc<Storage>,
    grant_id: i64,
    acceptor: tokio_rustls::TlsAcceptor,
) -> Result<()> {
    let AcceptedSession {
        session_id,
        mut send,
        recv,
        open_frame: _,
        leftover,
    } = session;

    // The relay's synthetic OPEN was already read + verified upstream.
    // Reply OPEN_ACK; only after this does either side emit a TLS byte
    // (`relay-protocol.md` "Inner-TLS wire shape" step 1).
    send.write_all(&encode_frame(FrameType::OpenAck, session_id, &[]))
        .await
        .context("write OPEN_ACK")?;

    let (inbound_tx, inbound_rx) = mpsc::channel::<Bytes>(SESSION_INBOUND_BUFFER);
    let send = Arc::new(Mutex::new(send));

    let send_for_reader = Arc::clone(&send);
    let reader = tokio::spawn(async move {
        if let Err(err) = pump_inbound(session_id, recv, leftover, inbound_tx, send_for_reader).await
        {
            debug!(target: "ohd_relay_client::responder", session_id, ?err, "inbound pump exited");
        }
    });

    let conn = SessionStream::new(session_id, inbound_rx, Arc::clone(&send));

    // Inner TLS 1.3 handshake — the storage is the TLS server, the consumer
    // the TLS client. The consumer's cert-pin check happens inside this
    // handshake; if it fails the consumer aborts and the handshake errors.
    let result = match acceptor.accept(conn).await {
        Ok(tls) => {
            debug!(target: "ohd_relay_client::responder", session_id, "inner TLS established");
            run_mcp_loop(tls, &storage, grant_id).await
        }
        Err(err) => {
            debug!(target: "ohd_relay_client::responder", session_id, ?err, "inner TLS handshake failed");
            Ok(())
        }
    };

    reader.abort();
    {
        let mut s = send.lock().await;
        let _ = s.write_all(&encode_frame(FrameType::Close, session_id, &[])).await;
        let _ = s.finish();
    }
    result
}

/// Pump frames off the QUIC `RecvStream` into the inbound mpsc, unwrapping
/// `DATA` payloads. A relay `CLOSE` ends the session.
async fn pump_inbound(
    session_id: u32,
    mut recv: quinn::RecvStream,
    mut seed: BytesMut,
    inbound_tx: mpsc::Sender<Bytes>,
    send: Arc<Mutex<quinn::SendStream>>,
) -> Result<()> {
    loop {
        loop {
            match decode_one_frame(&seed) {
                Ok((frame, consumed)) => {
                    let _ = seed.split_to(consumed);
                    match frame.frame_type {
                        FrameType::Data => {
                            if inbound_tx.send(frame.payload).await.is_err() {
                                return Ok(());
                            }
                        }
                        FrameType::Close => {
                            drop(inbound_tx);
                            let mut s = send.lock().await;
                            let _ = s
                                .write_all(&encode_frame(FrameType::Close, session_id, &[]))
                                .await;
                            let _ = s.finish();
                            return Ok(());
                        }
                        FrameType::OpenAck
                        | FrameType::OpenNack
                        | FrameType::WindowUpdate
                        | FrameType::Open
                        | FrameType::Hello
                        | FrameType::Ping
                        | FrameType::Pong => {
                            // Advisory / unexpected on a session stream — ignore.
                        }
                    }
                }
                Err(FrameError::Truncated) => break,
                Err(FrameError::Other(msg)) => anyhow::bail!("frame decode: {msg}"),
            }
        }
        let mut chunk = vec![0u8; 16 * 1024];
        match recv.read(&mut chunk).await? {
            Some(n) => seed.extend_from_slice(&chunk[..n]),
            None => return Ok(()),
        }
    }
}

// ===========================================================================
// Scoped MCP loop over the inner-TLS stream
// ===========================================================================

/// Speak newline-delimited MCP JSON-RPC over the established inner-TLS
/// session, scoped to the share's grant.
///
/// `initialize`, `tools/list`, `tools/call` are answered with the share's
/// [`ShareScope`]; everything else is `METHOD_NOT_FOUND`. The scope is
/// resolved **per request** from the live grant row, so a suspend / revoke
/// mid-session takes effect on the very next call.
async fn run_mcp_loop<S>(stream: S, storage: &Storage, grant_id: i64) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut lines = BufReader::new(read_half).lines();

    loop {
        let line = match read_line_bounded(&mut lines).await {
            Ok(Some(l)) => l,
            Ok(None) => break, // consumer closed the stream
            Err(err) => {
                warn!(target: "ohd_relay_client::responder", ?err, "MCP read error");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        let response = handle_mcp_request(&line, storage, grant_id);
        // A notification (no `id`) gets no reply.
        if let Some(resp) = response {
            let mut out = serde_json::to_vec(&resp).unwrap_or_default();
            out.push(b'\n');
            if write_half.write_all(&out).await.is_err() {
                break;
            }
            let _ = write_half.flush().await;
        }
    }
    let _ = write_half.shutdown().await;
    Ok(())
}

/// Read one newline-delimited JSON-RPC line, rejecting an over-long line.
async fn read_line_bounded<R>(
    lines: &mut tokio::io::Lines<BufReader<R>>,
) -> Result<Option<String>>
where
    R: AsyncRead + Unpin,
{
    match lines.next_line().await? {
        Some(line) if line.len() > MAX_MCP_LINE_BYTES => {
            anyhow::bail!("MCP request line exceeds {MAX_MCP_LINE_BYTES} bytes")
        }
        other => Ok(other),
    }
}

/// Dispatch one MCP JSON-RPC request line. Returns `None` for a
/// notification (a request with no `id`), `Some(response_json)` otherwise.
///
/// Resolves the share's live [`ShareScope`] fresh per request — a
/// mid-session suspend / revoke / expiry is honoured on the very next
/// call — then hands the parsed line + scope to the shared
/// [`ohd_mcp_core::wire::handle_json_rpc`] dispatcher. The wire layer
/// does all the protocol work; the responder just contributes scope and
/// transport (relay-tunnel-with-inner-TLS).
fn handle_mcp_request(line: &str, storage: &Storage, grant_id: i64) -> Option<Value> {
    let scope = match resolve_scope(storage, grant_id) {
        Ok(s) => s,
        Err(err) => {
            // Parse just enough of the envelope to mirror its id back on
            // the error response. JSON-RPC says transport errors get the
            // request's id when known, null otherwise.
            let id = serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|v| v.get("id").cloned())
                .unwrap_or(Value::Null);
            return Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32_603,
                    "message": format!("scope resolution failed: {err}"),
                },
            }));
        }
    };
    ohd_mcp_core::wire::handle_json_rpc(line, storage, Some(&scope), RESPONDER_SERVER_INFO)
}

/// Identifies this transport in MCP `initialize` responses. The phone
/// responder advertises itself separately from the SaaS storage server's
/// `/mcp` route so a connecting agent can tell which surface it reached.
const RESPONDER_SERVER_INFO: ohd_mcp_core::wire::ServerInfo = ohd_mcp_core::wire::ServerInfo {
    name: "ohd-share-responder",
    version: env!("CARGO_PKG_VERSION"),
};

/// Resolve the live [`ShareScope`] for `grant_id` off the storage core.
fn resolve_scope(storage: &Storage, grant_id: i64) -> Result<ShareScope> {
    let grant = storage
        .with_conn(|conn| ohd_storage_core::grants::read_grant(conn, grant_id))
        .context("read grant row")?;
    Ok(ShareScope::from_grant(&grant, now_ms()))
}

fn sha256(input: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(input);
    let mut out = [0u8; 32];
    out.copy_from_slice(&h.finalize());
    out
}

/// base64url-no-pad — the encoding the share artifact's `pin=` carries.
fn b64url_no_pad(bytes: &[u8]) -> String {
    // A tiny self-contained encoder keeps the responder off a `base64`
    // direct dep (the crate's other modules don't need one).
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity((bytes.len() * 4).div_ceil(3));
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 0x3F) as usize] as char);
        }
    }
    out
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ===========================================================================
// SessionStream: AsyncRead + AsyncWrite over the tunnel's DATA frames
// ===========================================================================

/// Bridges a relay session's `DATA`-frame envelope pair into a
/// `tokio::io::{AsyncRead, AsyncWrite}` so a `tokio_rustls` TLS server can
/// run over it. Inbound `DATA` payloads (already unwrapped by
/// [`pump_inbound`]) are concatenated into a byte stream; outbound writes
/// are wrapped one `DATA` frame at a time, chunked at
/// [`MAX_OUTBOUND_PAYLOAD_BYTES`].
struct SessionStream {
    session_id: u32,
    inbound: mpsc::Receiver<Bytes>,
    leftover: Bytes,
    send: Arc<Mutex<quinn::SendStream>>,
    pending_write: Option<Pin<Box<dyn Future<Output = io::Result<usize>> + Send>>>,
}

impl SessionStream {
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

impl AsyncRead for SessionStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if !self.leftover.is_empty() {
            let take = std::cmp::min(self.leftover.len(), out.remaining());
            let chunk = self.leftover.split_to(take);
            out.put_slice(&chunk);
            return Poll::Ready(Ok(()));
        }
        match self.inbound.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(Ok(())), // stream end → EOF
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

impl AsyncWrite for SessionStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        src: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Some(fut) = self.pending_write.as_mut() {
            return match fut.as_mut().poll(cx) {
                Poll::Pending => Poll::Pending,
                Poll::Ready(res) => {
                    self.pending_write = None;
                    Poll::Ready(res)
                }
            };
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
        match self.pending_write.as_mut().unwrap().as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(res) => {
                self.pending_write = None;
                Poll::Ready(res)
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        if let Some(fut) = self.pending_write.as_mut() {
            match fut.as_mut().poll(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Ok(_)) => self.pending_write = None,
                Poll::Ready(Err(err)) => {
                    self.pending_write = None;
                    return Poll::Ready(Err(err));
                }
            }
        }
        Poll::Ready(Ok(()))
    }
}

fn io_err<E: std::fmt::Display>(err: E) -> io::Error {
    io::Error::other(err.to_string())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use ohd_storage_core::{Storage, StorageConfig};
    use rustls::pki_types::ServerName;
    use tokio::io::{duplex, AsyncReadExt, AsyncWriteExt};

    /// Generate a fresh Ed25519 identity key in PKCS#8 DER form.
    fn fresh_identity_key() -> Vec<u8> {
        rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519)
            .expect("gen identity key")
            .serialize_der()
    }

    /// An in-memory storage core with one allow-glucose grant; returns the
    /// storage and the grant id.
    fn storage_with_grant() -> (Arc<Storage>, i64) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("t.db");
        let storage = Storage::open(StorageConfig {
            path,
            cipher_key: vec![],
            create_if_missing: true,
            create_mode: ohd_storage_core::format::DeploymentMode::Primary,
            create_user_ulid: None,
        })
        .expect("open storage");
        // Leak the tempdir so the file outlives the test body.
        std::mem::forget(dir);

        use ohd_storage_core::grants::{NewGrant, RuleEffect};
        let new_grant = NewGrant {
            grantee_label: "CORD".into(),
            grantee_kind: "agent".into(),
            approval_mode: "never_required".into(),
            default_action: RuleEffect::Deny,
            event_type_rules: vec![("measurement.glucose".into(), RuleEffect::Allow)],
            ..Default::default()
        };
        let (grant_id, _ulid) = storage
            .with_conn_mut(|conn| ohd_storage_core::grants::create_grant(conn, &new_grant))
            .expect("create grant");
        (Arc::new(storage), grant_id)
    }

    #[test]
    fn registration_pin_matches_the_cert_the_responder_presents() {
        // The pin `register_share_rendezvous` derives must equal the pin a
        // consumer derives from the cert `build_inner_tls_acceptor` uses —
        // otherwise the share artifact's `pin=` would not match the wire.
        let key = fresh_identity_key();
        let kp = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
            &rustls::pki_types::PrivatePkcs8KeyDer::from(key.clone()),
            &rcgen::PKCS_ED25519,
        )
        .unwrap();
        let derived_pin = b64url_no_pad(&sha256(&kp.public_key_der()));

        let cert = storage_identity_cert(&key, "relay.example.com/r/x", now_secs()).unwrap();
        assert_eq!(
            derived_pin,
            cert.pin_b64url(),
            "registration pin must equal the responder cert's pin"
        );
    }

    #[test]
    fn mcp_initialize_returns_protocol_version() {
        let (storage, grant_id) = storage_with_grant();
        let req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let resp = handle_mcp_request(req, &storage, grant_id).expect("response");
        assert_eq!(resp["result"]["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert_eq!(resp["id"], 1);
    }

    #[test]
    fn mcp_notification_gets_no_reply() {
        let (storage, grant_id) = storage_with_grant();
        let req = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(handle_mcp_request(req, &storage, grant_id).is_none());
    }

    #[test]
    fn tools_list_is_scoped_to_the_grant() {
        let (storage, grant_id) = storage_with_grant();
        let req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}"#;
        let resp = handle_mcp_request(req, &storage, grant_id).expect("response");
        let tools = resp["result"]["tools"].as_array().expect("tools array");
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        // Read tools present; operator + write tools omitted for this
        // read-only grant.
        assert!(names.contains(&"query_events"), "read tool listed");
        assert!(!names.contains(&"create_grant"), "operator tool omitted");
        assert!(!names.contains(&"log_food"), "write tool omitted (read-only grant)");
    }

    #[test]
    fn tools_call_out_of_scope_is_not_permitted() {
        let (storage, grant_id) = storage_with_grant();
        // symptom.headache is NOT in the grant (deny default) → NotPermitted.
        let req = r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"query_events","arguments":{"event_type":"symptom.headache"}}}"#;
        let resp = handle_mcp_request(req, &storage, grant_id).expect("response");
        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("not permitted"), "got: {text}");
    }

    #[test]
    fn tools_call_operator_tool_rejected() {
        let (storage, grant_id) = storage_with_grant();
        let req = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"create_grant","arguments":{}}}"#;
        let resp = handle_mcp_request(req, &storage, grant_id).expect("response");
        assert_eq!(resp["result"]["isError"], true);
        let text = resp["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("owner-only"), "got: {text}");
    }

    #[test]
    fn tools_call_in_scope_read_succeeds() {
        let (storage, grant_id) = storage_with_grant();
        let req = r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"query_events","arguments":{"event_type":"measurement.glucose"}}}"#;
        let resp = handle_mcp_request(req, &storage, grant_id).expect("response");
        // In-scope read of an empty store: not an error, just zero events.
        assert_eq!(resp["result"]["isError"], false);
    }

    #[test]
    fn unknown_method_is_method_not_found() {
        let (storage, grant_id) = storage_with_grant();
        let req = r#"{"jsonrpc":"2.0","id":6,"method":"does/not/exist"}"#;
        let resp = handle_mcp_request(req, &storage, grant_id).expect("response");
        assert_eq!(resp["error"]["code"], -32_601);
    }

    #[test]
    fn parse_error_replies_with_null_id() {
        let (storage, grant_id) = storage_with_grant();
        let resp = handle_mcp_request("{not json", &storage, grant_id).expect("response");
        assert_eq!(resp["error"]["code"], -32_700);
        assert!(resp["id"].is_null());
    }

    #[test]
    fn suspended_grant_denies_every_tool_call() {
        let (storage, grant_id) = storage_with_grant();
        storage
            .with_conn(|conn| ohd_storage_core::grants::set_grant_suspended(conn, grant_id, true))
            .expect("suspend");
        let req = r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"query_events","arguments":{"event_type":"measurement.glucose"}}}"#;
        let resp = handle_mcp_request(req, &storage, grant_id).expect("response");
        assert_eq!(resp["result"]["isError"], true, "suspended grant denies reads");
    }

    /// End-to-end: a real inner-TLS handshake (pinned consumer ↔ responder
    /// server) over an in-memory duplex stream, then a scoped MCP exchange.
    #[tokio::test]
    async fn inner_tls_then_scoped_mcp_round_trip() {
        let (storage, grant_id) = storage_with_grant();
        let identity_key = fresh_identity_key();
        let rzv_url = "relay.example.com/r/testrendezvous";

        let acceptor = build_inner_tls_acceptor(&identity_key, rzv_url).expect("acceptor");
        let ident = storage_identity_cert(&identity_key, rzv_url, now_secs()).unwrap();

        // In-memory transport standing in for the relay's DATA-frame path.
        let (server_io, client_io) = duplex(64 * 1024);

        // Responder side: terminate TLS, run the scoped MCP loop.
        let server = tokio::spawn(async move {
            let tls = acceptor.accept(server_io).await.expect("server TLS");
            run_mcp_loop(tls, &storage, grant_id).await.expect("mcp loop");
        });

        // Consumer side: pinned TLS client, then one MCP request.
        let client_config = tls_pin::pinned_client_config(ident.spki_sha256).unwrap();
        let connector = tokio_rustls::TlsConnector::from(Arc::new(client_config));
        let sni = ServerName::try_from("ohd-storage").unwrap();
        let mut tls = connector.connect(sni, client_io).await.expect("client TLS");

        tls.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
            .await
            .unwrap();
        tls.flush().await.unwrap();

        let mut buf = vec![0u8; 64 * 1024];
        let n = tls.read(&mut buf).await.unwrap();
        let line = std::str::from_utf8(&buf[..n]).unwrap();
        let resp: Value = serde_json::from_str(line.trim()).expect("parse mcp response");
        assert_eq!(resp["id"], 1);
        let tools = resp["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "query_events"));
        assert!(!tools.iter().any(|t| t["name"] == "create_grant"));

        drop(tls);
        let _ = server.await;
    }
}
