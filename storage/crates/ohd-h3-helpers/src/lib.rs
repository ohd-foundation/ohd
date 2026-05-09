//! HTTP/3 (QUIC) plumbing shared between the OHD storage server and the
//! OHD relay.
//!
//! Both binaries used to ship near-identical copies of:
//!
//! - cert loading (PEM-from-file + dev self-signed via rcgen),
//! - QUIC `ServerConfig` setup (rustls + ALPN `h3`),
//! - streaming request-body adapter ([`H3RequestBody`]),
//! - and the accept loop pattern.
//!
//! This crate is the dedup. Per-binary glue (Connect-RPC service dispatch
//! in storage, axum router dispatch in relay) lives in each binary's
//! `http3.rs`, but they delegate cert / config / body handling and the
//! accept loop to the helpers here.
//!
//! # Why in-binary HTTP/3 (vs. Caddy-fronted)
//!
//! Both binaries want a single static binary deployment, lower socket
//! count under high event load, and clean QUIC migration semantics for
//! mobile clients. See `storage/STATUS.md` "HTTP/3 (in-binary) — landed"
//! for the long version.

use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::Context as _;
use bytes::{Buf, Bytes};
use http_body::{Body, Frame};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

// Re-exports so consumers don't need to add direct deps on these crates
// when they only touch them via the helper surface.
pub use h3;
pub use h3_quinn;
pub use http;
pub use quinn;
pub use rustls;

/// Self-signed certificate + key for dev / test use.
///
/// Uses `rcgen` to mint a P-256 cert valid for `localhost` + `127.0.0.1`.
/// Production deployments must supply real materials via
/// [`load_pem_cert_key`].
pub fn dev_self_signed_cert(
) -> anyhow::Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    let subject_alt_names = vec!["localhost".to_string(), "127.0.0.1".to_string()];
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(subject_alt_names)
            .context("rcgen self-signed certificate")?;
    let cert_der: CertificateDer<'static> = cert.der().clone();
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));
    Ok((vec![cert_der], key_der))
}

/// Load a PEM-encoded cert chain + private key from disk.
///
/// `cert_path` may contain one or more PEM `CERTIFICATE` blocks (typical
/// Let's Encrypt `fullchain.pem`). `key_path` must contain a single
/// `PRIVATE KEY` (PKCS#8), `RSA PRIVATE KEY` (PKCS#1), or
/// `EC PRIVATE KEY` (SEC1) block.
///
/// Errors out cleanly if either file is missing, unreadable, or contains
/// no decodable material — production cert misconfiguration should fail
/// fast at startup, not produce a server with `unwrap_err`-style surprises
/// at request time.
pub fn load_pem_cert_key(
    cert_path: &Path,
    key_path: &Path,
) -> anyhow::Result<(Vec<CertificateDer<'static>>, PrivateKeyDer<'static>)> {
    use std::fs::File;
    use std::io::BufReader;

    let cert_file = File::open(cert_path)
        .with_context(|| format!("open cert file {}", cert_path.display()))?;
    let mut cert_reader = BufReader::new(cert_file);
    let cert_chain: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("parse cert PEM {}", cert_path.display()))?;
    if cert_chain.is_empty() {
        anyhow::bail!("no CERTIFICATE blocks found in {}", cert_path.display());
    }

    let key_file = File::open(key_path)
        .with_context(|| format!("open key file {}", key_path.display()))?;
    let mut key_reader = BufReader::new(key_file);
    // `private_key` accepts PKCS#8, PKCS#1, and SEC1; returns the first
    // one found. For an explicit error message rather than the helper's
    // generic "no key" we re-check after.
    let key = rustls_pemfile::private_key(&mut key_reader)
        .with_context(|| format!("parse key PEM {}", key_path.display()))?
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no PRIVATE KEY / RSA PRIVATE KEY / EC PRIVATE KEY block in {}",
                key_path.display()
            )
        })?;

    Ok((cert_chain, key))
}

/// Build a `quinn::ServerConfig` that advertises ALPN `h3` over a
/// rustls-backed TLS handshake.
///
/// We bypass `quinn::ServerConfig::with_single_cert` because that helper
/// doesn't expose the rustls `alpn_protocols` field; we need to set it to
/// `b"h3"` for clients to accept the negotiation. The shape of the
/// underlying TLS config matches what `with_single_cert` produces (TLS 1.3
/// only, `max_early_data_size = u32::MAX`, no client auth) — we just take
/// the construction into our own hands.
pub fn server_config(
    cert_chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> anyhow::Result<quinn::ServerConfig> {
    // Ring is the crypto provider both binaries pin in their Cargo.toml.
    // Install it as the process-default if no provider has been
    // registered yet — calling this unconditionally is safe (the second
    // call is a no-op).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut tls = rustls::ServerConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
        .with_no_client_auth()
        .with_single_cert(cert_chain, key)
        .context("build rustls::ServerConfig")?;
    tls.max_early_data_size = u32::MAX;
    tls.alpn_protocols = vec![b"h3".to_vec()];

    let quic_crypto: quinn::crypto::rustls::QuicServerConfig = tls
        .try_into()
        .context("convert rustls::ServerConfig → QuicServerConfig")?;
    Ok(quinn::ServerConfig::with_crypto(Arc::new(quic_crypto)))
}

/// Open a `quinn::Endpoint` with the given cert+key and ALPN `h3` set.
///
/// Used by both binaries' `serve` functions; factoring it out here means
/// callers don't need to know about `server_config` separately.
pub fn make_endpoint(
    addr: SocketAddr,
    cert_chain: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> anyhow::Result<quinn::Endpoint> {
    let qcfg = server_config(cert_chain, key)?;
    quinn::Endpoint::server(qcfg, addr).context("quinn::Endpoint::server")
}

// ---------------------------------------------------------------------------
// Body adapter
// ---------------------------------------------------------------------------

/// Body error type. `connectrpc::ConnectRpcService` (and axum) requires
/// `B::Error: std::error::Error + Send + Sync + 'static`, and that bound
/// is *not* satisfied by `Box<dyn StdError>` (the std-trait blanket impl
/// only kicks in for unboxed concrete types). `std::io::Error` is a
/// concrete `StdError` impl and is the simplest carrier; we wrap any h3
/// error in `ErrorKind::Other` with a string message.
pub type H3BodyError = std::io::Error;

/// Type-erased `Future` that produces the next chunk of an h3 request body.
///
/// We need this layer of indirection because `h3::server::RequestStream`'s
/// `recv_data` is an async method on `&mut self`, but `Body::poll_frame`
/// exposes a sync `Pin<&mut Self>`. We can't define a named future type
/// here because the concrete return type of `recv_data` is private to h3;
/// boxing the future erases the lifetime + type.
pub type BoxedRecvFuture =
    Pin<Box<dyn Future<Output = Result<Option<Bytes>, H3BodyError>> + Send + 'static>>;

/// Streaming request body backed by an [`h3::server::RequestStream`].
///
/// `connectrpc::ConnectRpcService::call` accepts any
/// `http_body::Body<Data = Bytes>`, and so does `axum::body::Body::new`.
/// This adapter pulls chunks from the h3 stream lazily inside
/// `poll_frame` so server-streaming RPCs (and large-payload uploads like
/// `AttachBlob`) don't have to fit in a single in-memory `Bytes`.
///
/// The recv half of the bidi stream lives in an `Arc<Mutex<…>>` so the
/// async-fn future we synthesise in `poll_frame` can drive it without
/// fighting the body's `Pin<&mut Self>` borrow. The lock is uncontended in
/// practice — only this body polls it — so the Mutex is essentially a Send
/// wrapper.
pub struct H3RequestBody {
    pending: Option<BoxedRecvFuture>,
    next: Box<dyn Fn() -> BoxedRecvFuture + Send + Sync>,
    finished: bool,
}

impl H3RequestBody {
    pub fn new<F>(next: F) -> Self
    where
        F: Fn() -> BoxedRecvFuture + Send + Sync + 'static,
    {
        Self {
            pending: None,
            next: Box::new(next),
            finished: false,
        }
    }
}

impl Body for H3RequestBody {
    type Data = Bytes;
    type Error = H3BodyError;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if self.finished {
            return Poll::Ready(None);
        }
        let me = &mut *self;
        if me.pending.is_none() {
            me.pending = Some((me.next)());
        }
        let Some(fut) = me.pending.as_mut() else {
            // Unreachable: we just set `pending` above when it was None.
            // Treat as end-of-stream rather than panicking.
            me.finished = true;
            return Poll::Ready(None);
        };
        match fut.as_mut().poll(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(err)) => {
                me.pending = None;
                me.finished = true;
                Poll::Ready(Some(Err(err)))
            }
            Poll::Ready(Ok(None)) => {
                me.pending = None;
                me.finished = true;
                Poll::Ready(None)
            }
            Poll::Ready(Ok(Some(bytes))) => {
                me.pending = None;
                Poll::Ready(Some(Ok(Frame::data(bytes))))
            }
        }
    }
}

/// Drain one `recv_data()` chunk from a held h3 `RequestStream` lock,
/// converting any error into the carrier [`H3BodyError`].
///
/// The chunk type that `h3::quic::RecvStream::recv_data` yields is opaque
/// (h3 0.0.x doesn't name it as a public type), so we accept any type
/// that implements `Buf`. Both binaries call this from inside the
/// [`H3RequestBody::new`] closure to keep the per-binary `handle_request`
/// short.
///
/// `recv_result` is the `Result<Option<impl Buf>, _>` produced by
/// `recv_data().await`. A typical call site looks like:
///
/// ```ignore
/// let next = || {
///     let s = Arc::clone(&stream);
///     Box::pin(async move {
///         let mut guard = s.lock().await;
///         convert_recv_chunk(guard.recv_data().await)
///     }) as ohd_h3_helpers::BoxedRecvFuture
/// };
/// ```
pub fn convert_recv_chunk<C, E>(
    recv_result: Result<Option<C>, E>,
) -> Result<Option<Bytes>, H3BodyError>
where
    C: Buf,
    E: std::fmt::Display,
{
    match recv_result {
        Ok(Some(mut chunk)) => {
            let remaining = chunk.remaining();
            let mut tmp = vec![0u8; remaining];
            chunk.copy_to_slice(&mut tmp);
            Ok(Some(Bytes::from(tmp)))
        }
        Ok(None) => Ok(None),
        Err(err) => Err(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("h3 recv_data: {err}"),
        )),
    }
}
