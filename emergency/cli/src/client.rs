//! Connect-RPC client transports for the OHDC service.
//!
//! Two transports are supported, mirroring `../../connect/cli/src/client.rs`:
//!
//! 1. **HTTP/2 over plaintext h2c** — `http://host:port` URLs. Uses
//!    [`connectrpc::client::Http2Connection`] + the codegen-emitted
//!    [`OhdcServiceClient`]. TLS termination is delegated to the deployment
//!    (Caddy) per `../../storage/STATUS.md` "Wire-format swap".
//!
//! 2. **HTTP/3 over QUIC** — `https+h3://host:port` URLs. A thin direct-h3
//!    client (`H3RawClient`) speaks Connect-Protocol unary + server-stream
//!    framing directly. `connectrpc 0.4` ships only `Http2Connection` so we
//!    bypass it on the client side for h3.
//!
//! [`OhdcClient`] is a single facade exposing the methods the emergency
//! CLI uses (`get_case`, `list_cases`, `query_events`, `audit_query`).
//! Other methods (PutEvents, etc.) are not exposed here — the emergency
//! CLI never writes events; it only reads + exports.

use std::pin::Pin;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use bytes::{Buf, Bytes, BytesMut};

use buffa::Message;
use connectrpc::client::{ClientConfig, Http2Connection, SharedHttp2Connection};

use crate::proto::ohdc::v0 as pb;
use crate::proto::ohdc::v0::OhdcServiceClient;

#[allow(clippy::large_enum_variant)]
pub enum OhdcTransport {
    Http2(OhdcServiceClient<SharedHttp2Connection>),
    Http3(H3RawClient),
}

pub struct OhdcClient {
    pub transport: OhdcTransport,
    pub storage_url: String,
}

impl OhdcClient {
    /// Connect to a running storage server.
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
                })
            }
            "https+h3" => {
                let rest = &storage_url[scheme_end + 3..];
                let h3 = H3RawClient::connect(rest, token, insecure_skip_verify)
                    .await
                    .with_context(|| format!("HTTP/3 connect to {storage_url}"))?;
                Ok(Self {
                    transport: OhdcTransport::Http3(h3),
                    storage_url: storage_url.to_string(),
                })
            }
            "https" => bail!(
                "v1 CLI doesn't speak HTTPS over HTTP/2 directly (TLS is deployment-side \
                 via Caddy per ../../storage/STATUS.md). Use http://host:port for h2c, or \
                 https+h3://host:port for in-binary HTTP/3."
            ),
            other => bail!("unsupported scheme {other:?}; use http:// or https+h3://"),
        }
    }

    pub async fn get_case(&self, req: pb::GetCaseRequest) -> Result<pb::Case> {
        match &self.transport {
            OhdcTransport::Http2(c) => {
                let resp = c.get_case(req).await.map_err(|e| anyhow!("GetCase: {e}"))?;
                Ok(resp.into_owned())
            }
            OhdcTransport::Http3(c) => c.get_case(req).await,
        }
    }

    /// `OhdcService.ListCases` — kept on the facade for future
    /// subcommand growth (e.g. listing the operator's open cases). Not
    /// currently called by any subcommand.
    #[allow(dead_code)]
    pub async fn list_cases(
        &self,
        req: pb::ListCasesRequest,
    ) -> Result<pb::ListCasesResponse> {
        match &self.transport {
            OhdcTransport::Http2(c) => {
                let resp = c
                    .list_cases(req)
                    .await
                    .map_err(|e| anyhow!("ListCases: {e}"))?;
                Ok(resp.into_owned())
            }
            OhdcTransport::Http3(c) => c.list_cases(req).await,
        }
    }

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

    pub async fn audit_query(
        &self,
        req: pb::AuditQueryRequest,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<pb::AuditEntry>> + Send>>> {
        match &self.transport {
            OhdcTransport::Http2(c) => {
                let mut stream = c
                    .audit_query(req)
                    .await
                    .map_err(|e| anyhow!("AuditQuery: {e}"))?;
                let mut out = Vec::new();
                while let Some(view) = stream
                    .message()
                    .await
                    .map_err(|e| anyhow!("audit stream: {e}"))?
                {
                    out.push(view.to_owned_message());
                }
                Ok(Box::pin(futures::stream::iter(out.into_iter().map(Ok))))
            }
            OhdcTransport::Http3(c) => c.audit_query(req).await,
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP/3 raw client (mirrors connect/cli/src/client.rs::H3RawClient)
// ---------------------------------------------------------------------------

pub struct H3RawClient {
    sni_host: String,
    bearer: String,
    send_request: Arc<tokio::sync::Mutex<h3::client::SendRequest<h3_quinn::OpenStreams, Bytes>>>,
    _drive_task: Arc<DriveTaskGuard>,
}

struct DriveTaskGuard(tokio::task::JoinHandle<()>);
impl Drop for DriveTaskGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

impl H3RawClient {
    pub async fn connect(
        host_port: &str,
        bearer: &str,
        insecure_skip_verify: bool,
    ) -> Result<Self> {
        let _ = rustls::crypto::ring::default_provider().install_default();

        let addr: std::net::SocketAddr = match host_port.parse() {
            Ok(a) => a,
            Err(_) => {
                let mut iter = tokio::net::lookup_host(host_port)
                    .await
                    .with_context(|| format!("resolve {host_port}"))?;
                iter.next()
                    .ok_or_else(|| anyhow!("no DNS results for {host_port}"))?
            }
        };

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

        let mut tls = if insecure_skip_verify {
            rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(InsecureVerifier {
                    provider: rustls::crypto::ring::default_provider(),
                }))
                .with_no_client_auth()
        } else {
            bail!(
                "production HTTP/3 verification is not yet wired (the CLI doesn't bundle \
                 a trust root store yet). Use --insecure-skip-verify against a dev cert \
                 for now."
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

    async fn server_stream<Req: Message, Resp: Message + Default + 'static>(
        &self,
        method: &str,
        request: Req,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<Resp>> + Send>>> {
        let req_body = Bytes::from(request.encode_to_vec());
        let mut framed = BytesMut::with_capacity(5 + req_body.len());
        framed.extend_from_slice(&[0u8]);
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

        let body = drain_body(&mut stream).await?;
        let messages = decode_connect_stream::<Resp>(body)
            .with_context(|| format!("decode {method} stream"))?;
        Ok(Box::pin(futures::stream::iter(
            messages.into_iter().map(Ok),
        )))
    }

    pub async fn get_case(&self, req: pb::GetCaseRequest) -> Result<pb::Case> {
        self.unary("ohdc.v0.OhdcService/GetCase", req).await
    }

    #[allow(dead_code)]
    pub async fn list_cases(
        &self,
        req: pb::ListCasesRequest,
    ) -> Result<pb::ListCasesResponse> {
        self.unary("ohdc.v0.OhdcService/ListCases", req).await
    }

    pub async fn query_events(
        &self,
        req: pb::QueryEventsRequest,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<pb::Event>> + Send>>> {
        self.server_stream("ohdc.v0.OhdcService/QueryEvents", req)
            .await
    }

    pub async fn audit_query(
        &self,
        req: pb::AuditQueryRequest,
    ) -> Result<Pin<Box<dyn futures::Stream<Item = Result<pb::AuditEntry>> + Send>>> {
        self.server_stream("ohdc.v0.OhdcService/AuditQuery", req)
            .await
    }
}

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
            let env = std::str::from_utf8(payload).unwrap_or("");
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
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}
