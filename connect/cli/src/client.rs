//! Connect-RPC client transports for the OHDC service.
//!
//! Two transports are supported:
//!
//! 1. **HTTP/2 over plaintext h2c** — the original v1 path. Uses
//!    [`connectrpc::client::Http2Connection`] + the codegen-emitted
//!    [`OhdcServiceClient`]. TLS termination is delegated to the deployment
//!    (Caddy) per `../storage/STATUS.md` "Wire-format swap"; this transport
//!    only speaks plaintext h2c on `http://` URLs.
//!
//! 2. **HTTP/3 over QUIC** — new in this pass. Talks directly to a server's
//!    HTTP/3 (`quinn` + `h3`) listener, bypassing connectrpc on the client
//!    side. This is **path A** from the follow-ups list (`STATUS.md`):
//!    `connectrpc 0.4` ships only `Http2Connection`, no public h3-backed
//!    `Connection` impl, so a thin direct-h3 wrapper is the simplest way to
//!    reach a server's QUIC port.
//!
//! [`OhdcClient`] is a single facade exposing the same five v1 methods
//! (`health`, `whoami`, `put_events`, `query_events`, `get_event_by_ulid`)
//! over either transport. The transport is selected at construction time
//! by URL scheme:
//!
//! - `http://host:port`   → HTTP/2 (h2c, plaintext)
//! - `https+h3://host:port` → HTTP/3 (QUIC, TLS via rustls-ring)
//!
//! For dev / tests, `--insecure-skip-verify` accepts self-signed certs
//! (mirrors the storage server's `dev_self_signed_cert`).

use std::pin::Pin;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};

use connectrpc::client::{ClientConfig, Http2Connection, SharedHttp2Connection};

use crate::proto::ohdc::v0::OhdcServiceClient;

/// Selected wire transport for the OHDC client.
#[allow(clippy::large_enum_variant)]
pub enum OhdcTransport {
    /// HTTP/2 (h2c) via connectrpc's built-in `Http2Connection`.
    Http2(OhdcServiceClient<SharedHttp2Connection>),
    /// HTTP/3 (QUIC) via a thin direct-h3 client. Bypasses connectrpc; the
    /// per-method methods on [`OhdcClient`] dispatch directly into
    /// [`H3RawClient`].
    Http3(H3RawClient),
}

/// The OHDC client + the bearer token, used by every command but `login`.
pub struct OhdcClient {
    pub transport: OhdcTransport,
    pub storage_url: String,
    /// Bearer token retained on the facade for convenience (e.g. when an
    /// upper layer wants to re-issue the same auth header on a fresh
    /// stream). Currently unread by `main.rs`, hence the `dead_code`
    /// allow.
    #[allow(dead_code)]
    pub bearer: String,
}

// ---- Transport-agnostic high-level methods.
//
// Both transports return owned `pb::*Response` messages (no
// `Response<…>::into_view()` indirection) so call sites in `main.rs`
// don't need to know which transport is in use.
impl OhdcClient {
    pub async fn health(&self, req: pb::HealthRequest) -> Result<pb::HealthResponse> {
        match &self.transport {
            OhdcTransport::Http2(c) => {
                let resp = c
                    .health(req)
                    .await
                    .map_err(|e| anyhow!("Health: {e}"))?;
                Ok(resp.into_owned())
            }
            OhdcTransport::Http3(c) => c.health(req).await,
        }
    }

    pub async fn who_am_i(&self, req: pb::WhoAmIRequest) -> Result<pb::WhoAmIResponse> {
        match &self.transport {
            OhdcTransport::Http2(c) => {
                let resp = c
                    .who_am_i(req)
                    .await
                    .map_err(|e| anyhow!("WhoAmI: {e}"))?;
                Ok(resp.into_owned())
            }
            OhdcTransport::Http3(c) => c.who_am_i(req).await,
        }
    }

    pub async fn put_events(
        &self,
        req: pb::PutEventsRequest,
    ) -> Result<pb::PutEventsResponse> {
        match &self.transport {
            OhdcTransport::Http2(c) => {
                let resp = c
                    .put_events(req)
                    .await
                    .map_err(|e| anyhow!("PutEvents: {e}"))?;
                Ok(resp.into_owned())
            }
            OhdcTransport::Http3(c) => c.put_events(req).await,
        }
    }

    /// `OhdcService.GetEventByUlid` — the CLI doesn't currently expose a
    /// subcommand that calls it directly (the `query` subcommand uses
    /// `QueryEvents`), but the method is on the facade for completeness
    /// and for future subcommand growth.
    #[allow(dead_code)]
    pub async fn get_event_by_ulid(
        &self,
        req: pb::GetEventByUlidRequest,
    ) -> Result<pb::Event> {
        match &self.transport {
            OhdcTransport::Http2(c) => {
                let resp = c
                    .get_event_by_ulid(req)
                    .await
                    .map_err(|e| anyhow!("GetEventByUlid: {e}"))?;
                Ok(resp.into_owned())
            }
            OhdcTransport::Http3(c) => c.get_event_by_ulid(req).await,
        }
    }

    /// Server-streaming `QueryEvents`. Returns a stream of owned events.
    /// Both transports collect into the same iterator-of-events shape so
    /// call sites can treat the two paths uniformly.
    pub async fn query_events(
        &self,
        req: pb::QueryEventsRequest,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<pb::Event>> + Send>>> {
        match &self.transport {
            OhdcTransport::Http2(c) => {
                let mut stream = c
                    .query_events(req)
                    .await
                    .map_err(|e| anyhow!("QueryEvents: {e}"))?;
                let mut out = Vec::new();
                while let Some(view) = stream
                    .message()
                    .await
                    .map_err(|e| anyhow!("query stream: {e}"))?
                {
                    out.push(view.to_owned_message());
                }
                Ok(Box::pin(futures::stream::iter(out.into_iter().map(Ok))))
            }
            OhdcTransport::Http3(c) => c.query_events(req).await,
        }
    }
}

impl OhdcClient {
    /// Connect to a running storage server.
    ///
    /// `storage_url` accepts:
    /// - `http://host:port` — plaintext h2c (HTTP/2 path).
    /// - `https+h3://host:port` — HTTP/3 / QUIC. Validates the server cert
    ///   against the system roots unless `insecure_skip_verify` is set.
    pub async fn connect(
        storage_url: &str,
        token: &str,
        insecure_skip_verify: bool,
    ) -> Result<Self> {
        let scheme_end = storage_url
            .find("://")
            .ok_or_else(|| anyhow!("invalid storage URL: {storage_url}"))?;
        let scheme = &storage_url[..scheme_end];

        match scheme {
            "http" => {
                let uri: http::Uri = storage_url
                    .parse()
                    .with_context(|| format!("invalid storage URL: {storage_url}"))?;
                let conn = Http2Connection::connect_plaintext(uri.clone())
                    .await
                    .with_context(|| format!("HTTP/2 connect to {uri}"))?
                    .shared(64);
                let config = ClientConfig::new(uri.clone())
                    .protocol(connectrpc::Protocol::Grpc)
                    .default_header("authorization", format!("Bearer {token}"));
                let inner = OhdcServiceClient::new(conn, config);
                Ok(Self {
                    transport: OhdcTransport::Http2(inner),
                    storage_url: storage_url.to_string(),
                    bearer: token.to_string(),
                })
            }
            "https+h3" => {
                // Strip the scheme to isolate `host:port`. quinn doesn't
                // parse URIs; we need an explicit `SocketAddr` + SNI host.
                let rest = &storage_url[scheme_end + 3..];
                let h3 = H3RawClient::connect(rest, token, insecure_skip_verify)
                    .await
                    .with_context(|| format!("HTTP/3 connect to {storage_url}"))?;
                Ok(Self {
                    transport: OhdcTransport::Http3(h3),
                    storage_url: storage_url.to_string(),
                    bearer: token.to_string(),
                })
            }
            "https" => bail!(
                "v1 CLI doesn't speak HTTPS over HTTP/2 directly (TLS is deployment-side \
                 via Caddy per ../storage/STATUS.md). Use http://host:port for h2c, or \
                 https+h3://host:port for in-binary HTTP/3."
            ),
            other => bail!(
                "unsupported scheme {other:?}; use http:// or https+h3://"
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP/3 raw client
// ---------------------------------------------------------------------------

use bytes::{Buf, Bytes, BytesMut};

use buffa::Message;

use crate::proto::ohdc::v0 as pb;

/// Parameters for an `H3RawClient`. Held inside the client so we can open
/// a fresh request stream per RPC without re-doing the QUIC handshake.
pub struct H3RawClient {
    /// `host:port` of the remote (used both as the QUIC dial target and as
    /// the SNI hostname for the rustls handshake — we strip the port for
    /// SNI when needed).
    sni_host: String,
    /// Bearer token used for every request.
    bearer: String,
    /// Mutex-guarded `SendRequest` handle. h3 0.0.8 requires `&mut self`
    /// for `send_request`, and we want the client to be cheap to clone /
    /// reuse across RPCs without re-handshaking — wrapping it serialises
    /// access cheaply.
    send_request: Arc<tokio::sync::Mutex<h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>>>,
    /// Drives the h3 connection. Held here so the connection doesn't drop
    /// out from under us; aborted on `Drop`.
    _drive_task: Arc<DriveTaskGuard>,
}

struct DriveTaskGuard(tokio::task::JoinHandle<()>);
impl Drop for DriveTaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl H3RawClient {
    /// Connect to `host:port`. `host` must be present even for IP
    /// destinations (rustls' SNI requires a name; for `127.0.0.1:N` we send
    /// `localhost`).
    pub async fn connect(
        host_port: &str,
        bearer: &str,
        insecure_skip_verify: bool,
    ) -> Result<Self> {
        // rustls 0.23 demands a CryptoProvider; ring is the project default.
        // Calling install_default unconditionally is safe (subsequent calls
        // no-op).
        let _ = rustls::crypto::ring::default_provider().install_default();

        // Resolve `host_port` → SocketAddr. We don't go through `http::Uri`
        // here because `quinn` wants an actual addr, and the user's URL
        // might be `127.0.0.1:18443` (no DNS).
        let addr: std::net::SocketAddr = match host_port.parse() {
            Ok(a) => a,
            Err(_) => {
                // Try DNS resolution.
                let mut iter = tokio::net::lookup_host(host_port)
                    .await
                    .with_context(|| format!("resolve {host_port}"))?;
                iter.next()
                    .ok_or_else(|| anyhow!("no DNS results for {host_port}"))?
            }
        };

        // Pick an SNI host: split off the port; use "localhost" for IP-only.
        let sni_host = match host_port.rsplit_once(':') {
            Some((host, _)) if !host.is_empty() => {
                if host.parse::<std::net::IpAddr>().is_ok() {
                    "localhost".to_string()
                } else {
                    host.to_string()
                }
            }
            _ => "localhost".to_string(),
        };

        // Build the rustls client config.
        let mut tls = if insecure_skip_verify {
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(InsecureVerifier {
                    provider: rustls::crypto::ring::default_provider(),
                }))
                .with_no_client_auth()
        } else {
            // Standard rustls trust roots come from `rustls-native-certs`
            // or `webpki-roots`. We deliberately avoid pulling either crate
            // here for the v1 demo path — production HTTPS use should
            // pass `insecure_skip_verify=false` against a server using a
            // public CA cert and the system cert store, which means we'd
            // need rustls-native-certs. Until that lands, refuse to
            // attempt verification and surface the misconfiguration.
            bail!(
                "production HTTP/3 verification is not yet wired (the CLI doesn't bundle \
                 a trust root store yet). Use --insecure-skip-verify against a dev cert \
                 for now; production mode lands once rustls-native-certs is pulled in."
            );
        };
        tls.alpn_protocols = vec![b"h3".to_vec()];

        let quic_client_cfg: quinn::crypto::rustls::QuicClientConfig =
            tls.try_into().map_err(|e| anyhow!("TLS13 → QuicClientConfig: {e}"))?;
        let client_config = quinn::ClientConfig::new(Arc::new(quic_client_cfg));
        let mut endpoint = quinn::Endpoint::client(std::net::SocketAddr::new(
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED),
            0,
        ))
        .context("quinn::Endpoint::client")?;
        endpoint.set_default_client_config(client_config);

        let connection = endpoint
            .connect(addr, &sni_host)
            .map_err(|e| anyhow!("quinn dial: {e}"))?
            .await
            .map_err(|e| anyhow!("h3 handshake: {e}"))?;

        let h3_quinn_conn = h3_quinn::Connection::new(connection);
        let (mut driver, send_request) = h3::client::new(h3_quinn_conn)
            .await
            .map_err(|e| anyhow!("h3 client init: {e}"))?;

        // Drive the h3 driver in the background. h3 requires its driver
        // future to keep running so background streams (control + QPACK)
        // make progress; if we drop it the connection silently stalls.
        let drive_task = tokio::spawn(async move {
            let _ = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
        });

        Ok(Self {
            sni_host,
            bearer: bearer.to_string(),
            send_request: Arc::new(tokio::sync::Mutex::new(send_request)),
            _drive_task: Arc::new(DriveTaskGuard(drive_task)),
        })
    }

    /// Send a unary Connect-Protocol POST and return the decoded response.
    ///
    /// Connect's binary unary wire is:
    /// - Request: `application/proto` POST whose body is the raw Protobuf
    ///   bytes (no length prefix, no gRPC framing).
    /// - Response: HTTP 200 + `application/proto` body = raw Protobuf
    ///   bytes; non-2xx with a Connect error envelope on errors.
    async fn unary<Req: Message, Resp: Message + Default>(
        &self,
        method: &str,
        request: Req,
    ) -> Result<Resp> {
        let req_body = Bytes::from(request.encode_to_vec());
        let req = http::Request::builder()
            .method(http::Method::POST)
            .uri(format!("https://{}/{method}", self.sni_host))
            .header("content-type", "application/proto")
            .header("connect-protocol-version", "1")
            .header("authorization", format!("Bearer {}", self.bearer))
            .body(())
            .map_err(|e| anyhow!("http::Request::builder: {e}"))?;

        let mut sr = self.send_request.lock().await;
        let mut stream = sr
            .send_request(req)
            .await
            .map_err(|e| anyhow!("h3 send_request: {e}"))?;
        drop(sr);
        stream
            .send_data(req_body)
            .await
            .map_err(|e| anyhow!("h3 send_data: {e}"))?;
        stream
            .finish()
            .await
            .map_err(|e| anyhow!("h3 finish: {e}"))?;

        let resp = stream
            .recv_response()
            .await
            .map_err(|e| anyhow!("h3 recv_response: {e}"))?;
        if resp.status() != http::StatusCode::OK {
            // Drain body for diagnostics. Connect-protocol error bodies
            // are JSON envelopes; we surface them as anyhow strings.
            let body = drain_body(&mut stream).await?;
            let payload = String::from_utf8_lossy(&body).to_string();
            bail!(
                "OHDC HTTP/3 {method} returned status {} — {payload}",
                resp.status()
            );
        }
        let body = drain_body(&mut stream).await?;
        let parsed = Resp::decode_from_slice(&body[..])
            .map_err(|e| anyhow!("decode response for {method}: {e}"))?;
        Ok(parsed)
    }

    /// Send a server-streaming Connect-Protocol POST and return a stream
    /// of decoded messages.
    ///
    /// Connect's streaming wire frames each message:
    /// - 1 byte flags (`0x00` data, `0x02` end-of-stream)
    /// - 4 bytes BE length
    /// - `length` bytes of payload
    ///
    /// On the data path, payload is the proto-encoded message. The
    /// `0x02` "end" frame's payload is a JSON envelope (`{}` on success,
    /// `{"error":{...}}` on failure).
    async fn server_stream<Req: Message, Resp: Message + Default + 'static>(
        &self,
        method: &str,
        request: Req,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<Resp>> + Send>>> {
        let req_body = Bytes::from(request.encode_to_vec());
        // Per Connect spec, server-streaming uses
        // `application/connect+proto` and the request body is itself
        // length-prefixed: 1-byte flags + 4-byte BE length + payload.
        let mut framed = BytesMut::with_capacity(5 + req_body.len());
        framed.extend_from_slice(&[0u8]); // flags
        framed.extend_from_slice(&(req_body.len() as u32).to_be_bytes());
        framed.extend_from_slice(&req_body);
        let framed = framed.freeze();

        let req = http::Request::builder()
            .method(http::Method::POST)
            .uri(format!("https://{}/{method}", self.sni_host))
            .header("content-type", "application/connect+proto")
            .header("connect-protocol-version", "1")
            .header("authorization", format!("Bearer {}", self.bearer))
            .body(())
            .map_err(|e| anyhow!("http::Request::builder: {e}"))?;

        let mut sr = self.send_request.lock().await;
        let mut stream = sr
            .send_request(req)
            .await
            .map_err(|e| anyhow!("h3 send_request: {e}"))?;
        drop(sr);
        stream
            .send_data(framed)
            .await
            .map_err(|e| anyhow!("h3 send_data: {e}"))?;
        stream
            .finish()
            .await
            .map_err(|e| anyhow!("h3 finish: {e}"))?;

        let resp = stream
            .recv_response()
            .await
            .map_err(|e| anyhow!("h3 recv_response: {e}"))?;
        if resp.status() != http::StatusCode::OK {
            let body = drain_body(&mut stream).await?;
            let payload = String::from_utf8_lossy(&body).to_string();
            bail!(
                "OHDC HTTP/3 {method} returned status {} — {payload}",
                resp.status()
            );
        }

        // Consume the streaming body, parsing length-prefixed Connect
        // frames as we go. We collect ALL the frames eagerly and emit them
        // through a stream — sufficient for v1 (small QueryEvents results)
        // and avoids the boxed-future-in-Stream juggling that lazy-pull
        // would require here. If a streaming-during-receive use case
        // arrives, swap to async_stream::stream! and pull recv_data()
        // chunks lazily.
        let body = drain_body(&mut stream).await?;
        let messages = decode_connect_stream::<Resp>(body)
            .with_context(|| format!("decode {method} stream"))?;
        Ok(Box::pin(futures::stream::iter(
            messages.into_iter().map(Ok),
        )))
    }

    // ---- Generated-client-shaped wrappers around the wire methods.
    // Method paths match what `connectrpc-build` emits for OhdcService:
    // `/{package}.{Service}/{Method}`.

    pub async fn health(&self, req: pb::HealthRequest) -> Result<pb::HealthResponse> {
        self.unary("ohdc.v0.OhdcService/Health", req).await
    }

    pub async fn who_am_i(&self, req: pb::WhoAmIRequest) -> Result<pb::WhoAmIResponse> {
        self.unary("ohdc.v0.OhdcService/WhoAmI", req).await
    }

    pub async fn put_events(
        &self,
        req: pb::PutEventsRequest,
    ) -> Result<pb::PutEventsResponse> {
        self.unary("ohdc.v0.OhdcService/PutEvents", req).await
    }

    #[allow(dead_code)]
    pub async fn get_event_by_ulid(
        &self,
        req: pb::GetEventByUlidRequest,
    ) -> Result<pb::Event> {
        self.unary("ohdc.v0.OhdcService/GetEventByUlid", req).await
    }

    pub async fn query_events(
        &self,
        req: pb::QueryEventsRequest,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<pb::Event>> + Send>>> {
        self.server_stream("ohdc.v0.OhdcService/QueryEvents", req)
            .await
    }
}

// ---------------------------------------------------------------------------
// Body helpers
// ---------------------------------------------------------------------------

/// Drain every chunk an h3 server sends on a request stream into one
/// `Bytes` buffer. h3 yields `impl Buf`; we copy each into a `BytesMut`.
async fn drain_body<C, B>(
    stream: &mut h3::client::RequestStream<C, B>,
) -> Result<Bytes>
where
    C: h3::quic::RecvStream,
{
    let mut buf = BytesMut::new();
    while let Some(mut chunk) = stream
        .recv_data()
        .await
        .map_err(|e| anyhow!("recv_data: {e}"))?
    {
        let remaining = chunk.remaining();
        let mut tmp = vec![0u8; remaining];
        chunk.copy_to_slice(&mut tmp);
        buf.extend_from_slice(&tmp);
    }
    Ok(buf.freeze())
}

/// Decode a Connect-Protocol streaming body into a Vec of decoded messages.
///
/// Wire format (Connect-Protocol, server-streaming):
///   [1 byte flags][4 bytes BE length][length bytes payload]
/// repeated. The final frame has flags & 0x02 set; its payload is a
/// JSON envelope (`{}` on success, `{"error":{...}}` on failure).
fn decode_connect_stream<M: Message + Default>(body: Bytes) -> Result<Vec<M>> {
    let mut out = Vec::new();
    let mut buf = &body[..];
    while !buf.is_empty() {
        if buf.len() < 5 {
            bail!("truncated frame header (< 5 bytes)");
        }
        let flags = buf[0];
        let len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
        if buf.len() < 5 + len {
            bail!("truncated frame body ({} of {} bytes)", buf.len() - 5, len);
        }
        let payload = &buf[5..5 + len];
        if flags & 0x02 != 0 {
            // End-of-stream frame: payload is JSON. Surface server errors.
            let env = std::str::from_utf8(payload).unwrap_or("");
            // A success envelope is the literal `{}` (or `{"metadata":...}`).
            // An error envelope contains `"error"`. Cheap substring detection
            // is sufficient — the server emits one of two shapes.
            if env.contains("\"error\"") {
                bail!("server returned error envelope: {env}");
            }
            break;
        } else {
            let msg = M::decode_from_slice(payload)
                .map_err(|e| anyhow!("decode stream frame: {e}"))?;
            out.push(msg);
        }
        buf = &buf[5 + len..];
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Insecure verifier — DEV / TEST ONLY. Accepts every server cert.
// Mirrors the test pattern in `storage/.../tests/end_to_end_http3.rs`.
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct InsecureVerifier {
    provider: rustls::crypto::CryptoProvider,
}

impl rustls::client::danger::ServerCertVerifier for InsecureVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(
        &self,
        _: &[u8],
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}
