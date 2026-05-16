//! Relay-tunnelled MCP transport.
//!
//! For a `kind = "relay"` data source — a phone-resident storage reachable
//! only through OHD Relay — CORD cannot speak plain HTTP. It must:
//!
//! 1. **Attach to the relay rendezvous.** Open a WebSocket to the relay's
//!    consumer-attach endpoint `wss://<relay>/v1/attach/<rendezvous_id>`.
//!    The relay assigns a `SESSION_ID`, sends `OPEN` to the phone, and
//!    thereafter forwards opaque `DATA` frames in both directions.
//! 2. **Handshake inner TLS 1.3, pinned.** Inside the tunnel, CORD is the
//!    TLS client and the phone is the TLS server. CORD verifies the
//!    storage cert's SPKI SHA-256 against the share link's `pin`
//!    (`ohd-h3-helpers::tls_pin::pinned_client_config`) — fail-closed on
//!    mismatch. ALPN is `ohd-mcp1`.
//! 3. **Run MCP JSON-RPC over that TLS session** — `initialize`,
//!    `tools/list`, `tools/call`.
//!
//! See `relay/spec/relay-protocol.md` "TLS-through-tunnel" and
//! `cord/spec/data-link.md` "CORD connecting a data source".
//!
//! # Wire framing of MCP over the tunnel
//!
//! The inner TLS session negotiates ALPN `ohd-mcp1`. MCP messages ride it
//! as **newline-delimited JSON-RPC** — one compact JSON object per line,
//! the conventional MCP stdio framing. The phone-side share responder
//! (Phase 4d) must terminate inner-TLS and frame MCP the same way; this
//! is the contract Phase 4f integration-tests end to end.
//!
//! # Tunnel <-> TLS bridge
//!
//! [`TunnelBridge`] adapts the relay WebSocket into an `AsyncRead +
//! AsyncWrite` byte stream so `tokio_rustls` can drive a TLS handshake
//! over it. Outbound bytes become `DATA`-frame payloads (split at
//! `MAX_PAYLOAD_LEN`); inbound `DATA` frames are concatenated back into a
//! flat byte stream — frame boundaries need not align with TLS records
//! (`relay-protocol.md` inner-TLS wire shape, point 2).

use crate::types::AgentError;
use bytes::BytesMut;
use futures_util::{SinkExt, StreamExt};
use ohd_h3_helpers::tls_pin::INNER_TLS_ALPN;
use ohd_relay_client::frame::{decode_one_frame, encode_frame, FrameError, FrameType};
use serde_json::{json, Value};
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// How long we wait for the relay tunnel + inner-TLS handshake before
/// giving up — a sleeping phone may need a push-wake round-trip.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a single MCP request/response exchange may take.
const RPC_TIMEOUT: Duration = Duration::from_secs(30);

/// Everything needed to dial one relay-bound data source.
///
/// `relay_host` is the share link's `relay=` value (with or without an
/// `https://`/`wss://` scheme); `rendezvous_id` and `pin` come from the
/// same link. `token` is the unsealed `ohdg_…` grant credential.
#[derive(Clone, Debug)]
pub struct RelayTarget {
    pub relay_host: String,
    pub rendezvous_id: String,
    /// `base64url`-encoded SHA-256 of the storage identity cert SPKI.
    /// A relay-bound source without a pin cannot be trusted — pinning is
    /// the entire trust anchor for a self-signed phone cert.
    pub pin: String,
    pub token: String,
}

impl RelayTarget {
    /// Compose the relay's consumer-attach WebSocket URL,
    /// `wss://<host>/v1/attach/<rendezvous_id>`.
    ///
    /// `relay_host` is normalised: an `https://` scheme becomes `wss://`,
    /// `http://` becomes `ws://`, a bare host gets `wss://`, and any path
    /// the link carried is dropped.
    pub fn attach_url(&self) -> Result<String, AgentError> {
        let h = self.relay_host.trim();
        let (scheme, rest) = if let Some(r) = h.strip_prefix("https://") {
            ("wss", r)
        } else if let Some(r) = h.strip_prefix("http://") {
            ("ws", r)
        } else if let Some(r) = h.strip_prefix("wss://") {
            ("wss", r)
        } else if let Some(r) = h.strip_prefix("ws://") {
            ("ws", r)
        } else {
            ("wss", h)
        };
        // Drop any path/query the relay host carried; keep authority only.
        let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
        if authority.is_empty() {
            return Err(AgentError::Mcp("relay host is empty".into()));
        }
        Ok(format!(
            "{scheme}://{authority}/v1/attach/{}",
            self.rendezvous_id
        ))
    }
}

/// A live MCP session over a pinned inner-TLS connection through the relay.
///
/// Construct one with [`RelaySession::connect`] (which also runs MCP
/// `initialize`), then issue [`RelaySession::rpc`] calls. The session owns
/// the tunnel; dropping it tears the relay session down.
pub struct RelaySession {
    tls: tokio_rustls::client::TlsStream<TunnelBridge>,
    next_id: AtomicI64,
    inbuf: BytesMut,
}

impl RelaySession {
    /// Open the relay tunnel, complete the pinned inner-TLS 1.3 handshake,
    /// and run the MCP `initialize` exchange. The returned session is ready
    /// for `tools/list` / `tools/call`.
    pub async fn connect(target: &RelayTarget) -> Result<Self, AgentError> {
        tokio::time::timeout(CONNECT_TIMEOUT, Self::connect_inner(target))
            .await
            .map_err(|_| AgentError::Mcp("relay tunnel connect timed out".into()))?
    }

    async fn connect_inner(target: &RelayTarget) -> Result<Self, AgentError> {
        // 1. Attach to the relay rendezvous over a WebSocket. The relay
        //    assigns the SESSION_ID and forwards OPEN to the phone; the
        //    consumer never picks a session id (it sends DATA with id 0,
        //    the relay stamps its own — see relay `run_consumer_attach`).
        let url = target.attach_url()?;
        let (ws, _resp) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| AgentError::Mcp(format!("relay attach ({url}): {e}")))?;
        let bridge = TunnelBridge::new(ws);

        // 2. Inner TLS 1.3, pinned to the share's storage-identity pin.
        //    The verifier is fail-closed: a mismatch aborts the handshake
        //    and surfaces as a clear "not who the share said" error.
        let tls_config = pinned_client_config_from_b64url(&target.pin)?;
        let connector = tokio_rustls::TlsConnector::from(Arc::new(tls_config));
        // The pinned verifier ignores the SAN entirely; any syntactically
        // valid server name satisfies `rustls`.
        let server_name = rustls::pki_types::ServerName::try_from("storage.ohd.invalid")
            .map_err(|e| AgentError::Mcp(format!("server name: {e}")))?;
        let tls = connector
            .connect(server_name, bridge)
            .await
            .map_err(|e| AgentError::Mcp(format!("inner TLS handshake (pin check): {e}")))?;

        // Confirm the inner ALPN landed on `ohd-mcp1` — the responder
        // negotiates it end-to-end; a mismatch means a wrong endpoint.
        {
            let (_, conn) = tls.get_ref();
            if let Some(alpn) = conn.alpn_protocol() {
                if alpn != INNER_TLS_ALPN {
                    return Err(AgentError::Mcp(format!(
                        "inner TLS negotiated ALPN {:?}, expected ohd-mcp1",
                        String::from_utf8_lossy(alpn)
                    )));
                }
            }
        }

        let mut session = Self {
            tls,
            next_id: AtomicI64::new(1),
            inbuf: BytesMut::new(),
        };

        // 3. MCP `initialize` over the inner-TLS session.
        session
            .rpc(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "cord-agent",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                }),
            )
            .await?;
        Ok(session)
    }

    /// Issue one MCP JSON-RPC call over the inner-TLS session and return
    /// its `result`. The grant token rides the `params` (`_meta.token`) so
    /// the phone responder can re-validate it inside TLS — the relay's
    /// OPEN-payload token is only a cheap pre-screen.
    pub async fn rpc(&mut self, method: &str, params: Value) -> Result<Value, AgentError> {
        tokio::time::timeout(RPC_TIMEOUT, self.rpc_inner(method, params))
            .await
            .map_err(|_| AgentError::Mcp(format!("{method}: relay MCP call timed out")))?
    }

    async fn rpc_inner(&mut self, method: &str, params: Value) -> Result<Value, AgentError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        // Newline-delimited JSON-RPC: one compact object, one `\n`.
        let mut line = serde_json::to_vec(&req)
            .map_err(|e| AgentError::Mcp(format!("{method}: encode request: {e}")))?;
        line.push(b'\n');
        self.tls
            .write_all(&line)
            .await
            .map_err(|e| AgentError::Mcp(format!("{method}: tunnel write: {e}")))?;
        self.tls
            .flush()
            .await
            .map_err(|e| AgentError::Mcp(format!("{method}: tunnel flush: {e}")))?;

        let body = self.read_line(method).await?;
        if let Some(err) = body.get("error") {
            return Err(AgentError::Mcp(format!("{method}: {err}")));
        }
        Ok(body.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Read one newline-delimited JSON object off the inner-TLS stream.
    async fn read_line(&mut self, method: &str) -> Result<Value, AgentError> {
        let mut chunk = [0u8; 8 * 1024];
        loop {
            if let Some(pos) = self.inbuf.iter().position(|&b| b == b'\n') {
                let line = self.inbuf.split_to(pos + 1);
                let trimmed = &line[..line.len() - 1];
                if trimmed.iter().all(|b| b.is_ascii_whitespace()) {
                    continue;
                }
                return serde_json::from_slice(trimmed)
                    .map_err(|e| AgentError::Mcp(format!("{method}: response not JSON: {e}")));
            }
            let n = self
                .tls
                .read(&mut chunk)
                .await
                .map_err(|e| AgentError::Mcp(format!("{method}: tunnel read: {e}")))?;
            if n == 0 {
                return Err(AgentError::Mcp(format!(
                    "{method}: relay tunnel closed before a response arrived"
                )));
            }
            self.inbuf.extend_from_slice(&chunk[..n]);
        }
    }
}

/// Build the pinned inner-TLS `ClientConfig` from a share-link `pin`.
///
/// Delegates the SPKI-pin trust anchor entirely to
/// `ohd-h3-helpers::tls_pin` — the same fail-closed verifier the relay
/// spec mandates and the storage server / Android binding use. Maps the
/// helper's `anyhow` error onto [`AgentError`] so the call site reads
/// cleanly.
fn pinned_client_config_from_b64url(pin: &str) -> Result<rustls::ClientConfig, AgentError> {
    ohd_h3_helpers::tls_pin::pinned_client_config_b64url(pin.trim())
        .map_err(|e| AgentError::Mcp(format!("share-link pin is invalid: {e}")))
}

// ---------------------------------------------------------------------------
// TunnelBridge — relay WebSocket adapted to AsyncRead + AsyncWrite
// ---------------------------------------------------------------------------

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Adapts the relay's consumer-attach WebSocket into a byte stream.
///
/// `tokio_rustls` drives a TLS handshake over an `AsyncRead + AsyncWrite`;
/// the relay tunnel is a sequence of binary `TunnelFrame`s. This bridge
/// converts between the two:
///
/// - **write**: bytes the TLS engine emits are wrapped in `DATA` frames
///   (split at `MAX_PAYLOAD_LEN`) and sent as binary WebSocket messages.
/// - **read**: inbound binary WebSocket messages are decoded as frames;
///   `DATA` payloads are concatenated into `read_buf`. `CLOSE` ends the
///   stream; `OPEN_ACK` / `PING` / `PONG` / `WINDOW_UPDATE` are accepted
///   and skipped (no flow-control or keepalive needed for short MCP
///   sessions). `OPEN_NACK` surfaces as a hard error.
///
/// The consumer always tags outbound frames with `SESSION_ID = 0`; the
/// relay overwrites it with the real id (`run_consumer_attach` in
/// `relay/src/server.rs`), so the bridge never needs to learn it.
pub struct TunnelBridge {
    ws: Ws,
    /// Decoded-but-unconsumed inbound DATA payload bytes.
    read_buf: BytesMut,
    /// Raw inbound WebSocket bytes not yet decoded into a full frame.
    frame_buf: BytesMut,
    /// Set once the peer/relay closed the tunnel.
    eof: bool,
}

impl TunnelBridge {
    fn new(ws: Ws) -> Self {
        Self {
            ws,
            read_buf: BytesMut::new(),
            frame_buf: BytesMut::new(),
            eof: false,
        }
    }
}

/// Decode every complete `TunnelFrame` sitting in `frame_buf`, appending
/// `DATA` payloads to `read_buf`. Returns `Ok(true)` if a `CLOSE` was seen
/// (stream EOF). Non-DATA control frames are skipped; `OPEN_NACK` errors.
fn drain_frames(frame_buf: &mut BytesMut, read_buf: &mut BytesMut) -> io::Result<bool> {
    let mut closed = false;
    loop {
        match decode_one_frame(frame_buf) {
            Ok((frame, consumed)) => {
                let _ = frame_buf.split_to(consumed);
                match frame.frame_type {
                    FrameType::Data => read_buf.extend_from_slice(&frame.payload),
                    FrameType::Close => {
                        closed = true;
                        break;
                    }
                    FrameType::OpenNack => {
                        let reason = String::from_utf8_lossy(&frame.payload).into_owned();
                        return Err(io::Error::new(
                            io::ErrorKind::ConnectionRefused,
                            format!("relay OPEN_NACK: {reason}"),
                        ));
                    }
                    // OPEN_ACK / PING / PONG / WINDOW_UPDATE / HELLO / OPEN:
                    // accepted, nothing to do for a short MCP session.
                    _ => {}
                }
            }
            Err(FrameError::Truncated) => break,
            Err(FrameError::Other(msg)) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("relay frame decode: {msg}"),
                ));
            }
        }
    }
    Ok(closed)
}

impl AsyncRead for TunnelBridge {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            // Serve anything already decoded.
            if !self.read_buf.is_empty() {
                let n = self.read_buf.len().min(buf.remaining());
                let chunk = self.read_buf.split_to(n);
                buf.put_slice(&chunk);
                return Poll::Ready(Ok(()));
            }
            if self.eof {
                return Poll::Ready(Ok(())); // clean EOF
            }

            // Try to decode whatever raw bytes we already have buffered.
            {
                let mut frame_buf = std::mem::take(&mut self.frame_buf);
                let mut read_buf = std::mem::take(&mut self.read_buf);
                match drain_frames(&mut frame_buf, &mut read_buf) {
                    Ok(closed) => {
                        self.frame_buf = frame_buf;
                        self.read_buf = read_buf;
                        if closed {
                            self.eof = true;
                            continue;
                        }
                        if !self.read_buf.is_empty() {
                            continue;
                        }
                    }
                    Err(e) => return Poll::Ready(Err(e)),
                }
            }

            // Pull the next WebSocket message.
            match self.ws.poll_next_unpin(cx) {
                Poll::Ready(Some(Ok(msg))) => match msg {
                    WsMessage::Binary(bytes) => {
                        self.frame_buf.extend_from_slice(&bytes);
                        // Loop: decode what we just got.
                        continue;
                    }
                    WsMessage::Close(_) => {
                        self.eof = true;
                        continue;
                    }
                    // Text / Ping / Pong / Frame: irrelevant to the tunnel.
                    _ => continue,
                },
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Err(io::Error::other(format!(
                        "relay websocket read: {e}"
                    ))));
                }
                Poll::Ready(None) => {
                    self.eof = true;
                    continue;
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

impl AsyncWrite for TunnelBridge {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        // Wrap the TLS engine's bytes into one DATA frame per chunk. The
        // relay stamps the SESSION_ID, so the consumer sends 0.
        let chunk = encode_data_frames(data);
        match self.ws.poll_ready_unpin(cx) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(e)) => {
                return Poll::Ready(Err(io::Error::other(format!(
                    "relay websocket not ready: {e}"
                ))));
            }
            Poll::Pending => return Poll::Pending,
        }
        match self.ws.start_send_unpin(WsMessage::Binary(chunk)) {
            Ok(()) => Poll::Ready(Ok(data.len())),
            Err(e) => Poll::Ready(Err(io::Error::other(format!(
                "relay websocket send: {e}"
            )))),
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.ws
            .poll_flush_unpin(cx)
            .map_err(|e| io::Error::other(format!("relay websocket flush: {e}")))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.ws
            .poll_close_unpin(cx)
            .map_err(|e| io::Error::other(format!("relay websocket close: {e}")))
    }
}

/// Encode `data` as one or more `DATA` frames (split at `MAX_PAYLOAD_LEN`),
/// concatenated into a single buffer ready for one binary WebSocket
/// message. `SESSION_ID` is `0` — the relay overwrites it.
fn encode_data_frames(data: &[u8]) -> Vec<u8> {
    use ohd_relay_client::frame::MAX_PAYLOAD_LEN;
    let mut out = Vec::with_capacity(data.len() + 16);
    for chunk in data.chunks(MAX_PAYLOAD_LEN.max(1)) {
        out.extend_from_slice(&encode_frame(FrameType::Data, 0, chunk));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohd_relay_client::frame::{encode_frame, FrameType};

    fn target(host: &str) -> RelayTarget {
        RelayTarget {
            relay_host: host.into(),
            rendezvous_id: "RV123".into(),
            pin: "AAAA".into(),
            token: "ohdg_x".into(),
        }
    }

    #[test]
    fn attach_url_https_becomes_wss() {
        let u = target("https://relay.ohd.dev").attach_url().unwrap();
        assert_eq!(u, "wss://relay.ohd.dev/v1/attach/RV123");
    }

    #[test]
    fn attach_url_bare_host_defaults_wss() {
        let u = target("relay.ohd.dev:9443").attach_url().unwrap();
        assert_eq!(u, "wss://relay.ohd.dev:9443/v1/attach/RV123");
    }

    #[test]
    fn attach_url_http_becomes_ws() {
        let u = target("http://127.0.0.1:8080").attach_url().unwrap();
        assert_eq!(u, "ws://127.0.0.1:8080/v1/attach/RV123");
    }

    #[test]
    fn attach_url_drops_trailing_path() {
        let u = target("https://relay.ohd.dev/some/path")
            .attach_url()
            .unwrap();
        assert_eq!(u, "wss://relay.ohd.dev/v1/attach/RV123");
    }

    #[test]
    fn attach_url_already_wss_kept() {
        let u = target("wss://relay.ohd.dev").attach_url().unwrap();
        assert_eq!(u, "wss://relay.ohd.dev/v1/attach/RV123");
    }

    #[test]
    fn attach_url_empty_host_rejected() {
        assert!(target("https://").attach_url().is_err());
    }

    /// A real, well-formed `base64url` pin: SHA-256 of a freshly-minted
    /// storage identity cert's SPKI — the exact shape a share link carries.
    fn real_pin() -> String {
        use ohd_h3_helpers::tls_pin::storage_identity_cert;
        let kp = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).expect("identity key");
        let ident = storage_identity_cert(&kp.serialize_der(), "relay.ohd.dev/r/RV123", 1_770_000_000)
            .expect("mint identity cert");
        ident.pin_b64url()
    }

    #[test]
    fn pinned_config_built_from_real_pin() {
        // The pin from a share link feeds straight into the fail-closed
        // verifier; the resulting config must carry the inner-TLS ALPN.
        let cfg = pinned_client_config_from_b64url(&real_pin()).expect("config");
        assert_eq!(cfg.alpn_protocols, vec![INNER_TLS_ALPN.to_vec()]);
    }

    #[test]
    fn pinned_config_rejects_bad_pin() {
        // Too short to be a 32-byte SHA-256 — must fail closed, not build.
        assert!(pinned_client_config_from_b64url("xx").is_err());
        assert!(pinned_client_config_from_b64url("not base64!!!").is_err());
    }

    #[test]
    fn drain_frames_concatenates_data_payloads() {
        let mut frame_buf = BytesMut::new();
        frame_buf.extend_from_slice(&encode_frame(FrameType::Data, 0, b"hello "));
        frame_buf.extend_from_slice(&encode_frame(FrameType::Data, 0, b"world"));
        let mut read_buf = BytesMut::new();
        let closed = drain_frames(&mut frame_buf, &mut read_buf).unwrap();
        assert!(!closed);
        assert_eq!(&read_buf[..], b"hello world");
        assert!(frame_buf.is_empty());
    }

    #[test]
    fn drain_frames_skips_open_ack_and_window_update() {
        let mut frame_buf = BytesMut::new();
        frame_buf.extend_from_slice(&encode_frame(FrameType::OpenAck, 0, &[]));
        frame_buf.extend_from_slice(&encode_frame(FrameType::Data, 0, b"x"));
        frame_buf.extend_from_slice(&encode_frame(FrameType::WindowUpdate, 0, &[]));
        let mut read_buf = BytesMut::new();
        let closed = drain_frames(&mut frame_buf, &mut read_buf).unwrap();
        assert!(!closed);
        assert_eq!(&read_buf[..], b"x");
    }

    #[test]
    fn drain_frames_reports_close() {
        let mut frame_buf = BytesMut::new();
        frame_buf.extend_from_slice(&encode_frame(FrameType::Data, 0, b"tail"));
        frame_buf.extend_from_slice(&encode_frame(FrameType::Close, 0, &[]));
        let mut read_buf = BytesMut::new();
        let closed = drain_frames(&mut frame_buf, &mut read_buf).unwrap();
        assert!(closed);
        assert_eq!(&read_buf[..], b"tail");
    }

    #[test]
    fn drain_frames_open_nack_is_error() {
        let mut frame_buf = BytesMut::new();
        frame_buf.extend_from_slice(&encode_frame(FrameType::OpenNack, 0, b"INVALID_TOKEN"));
        let mut read_buf = BytesMut::new();
        let err = drain_frames(&mut frame_buf, &mut read_buf).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionRefused);
        assert!(err.to_string().contains("INVALID_TOKEN"));
    }

    #[test]
    fn drain_frames_leaves_partial_frame() {
        let whole = encode_frame(FrameType::Data, 0, b"partial");
        let mut frame_buf = BytesMut::new();
        frame_buf.extend_from_slice(&whole[..whole.len() - 3]); // truncated
        let mut read_buf = BytesMut::new();
        let closed = drain_frames(&mut frame_buf, &mut read_buf).unwrap();
        assert!(!closed);
        assert!(read_buf.is_empty());
        assert_eq!(frame_buf.len(), whole.len() - 3); // preserved for next read
    }

    #[test]
    fn encode_data_frames_splits_oversized_payload() {
        use ohd_relay_client::frame::{decode_one_frame, FRAME_HEADER_LEN, MAX_PAYLOAD_LEN};
        let big = vec![7u8; MAX_PAYLOAD_LEN + 100];
        let out = encode_data_frames(&big);
        // Two frames: a full one and a 100-byte remainder.
        let (f1, c1) = decode_one_frame(&out).unwrap();
        assert_eq!(f1.payload.len(), MAX_PAYLOAD_LEN);
        let (f2, c2) = decode_one_frame(&out[c1..]).unwrap();
        assert_eq!(f2.payload.len(), 100);
        assert_eq!(c1 + c2, out.len());
        assert_eq!(c1, FRAME_HEADER_LEN + MAX_PAYLOAD_LEN);
    }
}
