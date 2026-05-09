//! HTTP/3 (QUIC) listener for the OHDC Connect-RPC service.
//!
//! Why this exists (in-binary HTTP/3 vs Caddy-fronted):
//!
//! * **Single static binary deployment** — the project ships as one binary so
//!   operators don't have to run + monitor Caddy alongside it. HTTP/3 in
//!   process keeps "deploy in an afternoon" honest.
//! * **Socket count under high event load** — large sensor / EHR fleets push
//!   continuous device-token writes (Libre, Dexcom, Garmin, lab providers,
//!   …); QUIC's connection migration + cheap stream suspend/resume scales
//!   better than thousands of TCP sockets.
//! * **Mobile clients** — Connect on iOS/Android over flaky cellular: QUIC's
//!   connection-migration + 0-RTT recovery is genuinely better than TCP.
//!
//! # Design
//!
//! `connectrpc 0.4` exposes [`ConnectRpcService`] (a transport-agnostic
//! `tower::Service<http::Request<B>>`); the HTTP/2 path mounts it via
//! `into_axum_service()`, this module mounts it on top of `quinn` + `h3`.
//!
//! Per-connection: a `quinn::Endpoint` hands us a [`quinn::Connection`],
//! which `h3_quinn::Connection::new()` wraps for the [`h3::server`] driver.
//! Per-request: we receive headers + body via h3's `RequestStream`,
//! materialize the body as a streaming [`H3RequestBody`] (which implements
//! `http_body::Body<Data = Bytes>` and pulls chunks from
//! `RequestStream::recv_data` on demand), call `service.call(http::Request)`,
//! and pump the response body's frames back via `send_data` / `send_trailers`.
//!
//! ## Shared primitives
//!
//! Cert loading (PEM-from-file + dev self-signed via rcgen), the QUIC
//! `ServerConfig` setup, and the [`H3RequestBody`] adapter all live in the
//! workspace's `ohd-h3-helpers` crate so the relay's mirror file can use
//! the exact same plumbing. Only the per-request dispatch glue (calling
//! `ConnectRpcService::clone()` and pumping its response body back through
//! the h3 stream) stays in this binary.
//!
//! ## Streaming body adapter
//!
//! Earlier passes buffered the entire request body to a single `Bytes` (4 MiB
//! cap) before dispatching. That worked for unary RPCs but blocked the
//! streaming RPCs (`AttachBlob`, `Import`, `QueryEvents`, …) from running on
//! HTTP/3. The body adapter ([`H3RequestBody`]) walks `recv_data` one chunk
//! at a time and yields each as a `Frame::data`, matching the unbuffered
//! semantics the HTTP/2 path already has.
//!
//! # Trailers and Connect protocol
//!
//! Connect's own protocol (`application/proto` for unary,
//! `application/connect+proto` for streaming) returns the trailing status as
//! a body envelope, **not** as HTTP trailers. gRPC + gRPC-Web do use
//! trailers. h3 0.0.8 supports trailers via `RequestStream::send_trailers`,
//! so we forward them when the response body produces a `Frame::trailers`.
//!
//! # Production cert flags
//!
//! [`load_pem_cert_key`] reads a PEM-encoded cert chain + private key from
//! disk via `rustls-pemfile 2`. The `serve` subcommand wires
//! `--http3-cert PATH` / `--http3-key PATH` through to this helper; when the
//! flags are absent the loader falls back to [`dev_self_signed_cert`] and
//! emits a warning to stderr (production deployments must supply real
//! materials).

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Context as _;
use bytes::Bytes;
use connectrpc::ConnectRpcService;
use http::Request;
use http_body::Body;
use tokio::sync::Mutex;
use tower::Service;

// Re-export the shared helpers under their original names so call sites
// that look up `http3::load_pem_cert_key` etc. (e.g. main.rs) keep
// working unchanged. `server_config` and `H3BodyError` are part of the
// public surface even when unused in this file — they're consumed by
// the relay's mirror crate path.
#[allow(unused_imports)]
pub use ohd_h3_helpers::{
    convert_recv_chunk, dev_self_signed_cert, load_pem_cert_key, make_endpoint, server_config,
    BoxedRecvFuture, H3BodyError, H3RequestBody,
};

/// Run the HTTP/3 (QUIC) listener until cancelled.
///
/// `addr` is the UDP bind address. `service` is the same
/// [`ConnectRpcService`] the HTTP/2 path mounts; this listener clones it
/// per request.
///
/// The listener spawns one task per accepted connection (`h3` driver), and
/// one task per accepted request (the request handler). Errors at either
/// layer are logged, not bubbled — a single bad connection should not take
/// the listener down.
pub async fn serve(
    addr: SocketAddr,
    service: ConnectRpcService,
    cert_chain: Vec<rustls::pki_types::CertificateDer<'static>>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
) -> anyhow::Result<()> {
    let endpoint = make_endpoint(addr, cert_chain, key)?;
    tracing::info!(%addr, "OHDC Connect-RPC HTTP/3 listening (QUIC)");

    while let Some(incoming) = endpoint.accept().await {
        let svc = service.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(incoming, svc).await {
                tracing::warn!(?err, "h3 connection ended with error");
            }
        });
    }
    Ok(())
}

async fn handle_connection(
    incoming: quinn::Incoming,
    service: ConnectRpcService,
) -> anyhow::Result<()> {
    let conn = incoming.await.context("quinn handshake")?;
    tracing::debug!(remote = %conn.remote_address(), "h3 connection accepted");

    let mut h3_conn = h3::server::Connection::<_, Bytes>::new(h3_quinn::Connection::new(conn))
        .await
        .context("h3 server connection")?;

    loop {
        match h3_conn.accept().await {
            Ok(Some(resolver)) => {
                let svc = service.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_request(resolver, svc).await {
                        tracing::warn!(?err, "h3 request error");
                    }
                });
            }
            Ok(None) => break,
            Err(err) => {
                // Connection-level error: log and stop accepting on this
                // connection. Other connections continue.
                tracing::debug!(?err, "h3 accept error");
                break;
            }
        }
    }
    Ok(())
}

/// Handle a single HTTP/3 request: build an `http::Request`, call into the
/// shared `ConnectRpcService`, and pump the response back through h3.
///
/// The request body streams through [`H3RequestBody`] (no 4 MiB buffer cap);
/// the response body's frames are written back via `send_data` /
/// `send_trailers` chunk-by-chunk.
async fn handle_request<C>(
    resolver: h3::server::RequestResolver<C, Bytes>,
    mut service: ConnectRpcService,
) -> anyhow::Result<()>
where
    C: h3::quic::Connection<Bytes>,
    <C as h3::quic::OpenStreams<Bytes>>::BidiStream: Send + 'static,
{
    let (req, stream) = resolver
        .resolve_request()
        .await
        .context("h3 resolve_request")?;

    // Wrap the full bidi `RequestStream` in an Arc<Mutex<…>> so two halves of
    // the request lifecycle can both drive it: the body future (recv_data)
    // and the response writer (send_response / send_data / send_trailers).
    // The lock is uncontended in practice — request-body recvs happen before
    // the response writer engages, and the response writer holds the lock
    // for the duration of each send.
    let stream = Arc::new(Mutex::new(stream));

    // Build the streaming request body. The `next` closure captures a clone
    // of the Arc so each poll spawns a fresh `recv_data` future against the
    // shared stream. The chunk-conversion glue lives in `ohd-h3-helpers`.
    let body_stream = Arc::clone(&stream);
    let body = H3RequestBody::new(move || -> BoxedRecvFuture {
        let s = Arc::clone(&body_stream);
        Box::pin(async move {
            let mut guard = s.lock().await;
            convert_recv_chunk(guard.recv_data().await)
        })
    });

    // h3 hands us a `Request<()>`. Re-build it with our streaming body.
    let (parts, ()) = req.into_parts();
    let request: Request<H3RequestBody> = Request::from_parts(parts, body);

    // ---- Dispatch through ConnectRpcService. ----
    //
    // ConnectRpcService::call returns `Future<Output = Result<Response<ConnectRpcBody>, Infallible>>`,
    // so the unwrap below is sound — Infallible can't be constructed.
    let response = service
        .call(request)
        .await
        .unwrap_or_else(|never| match never {});

    // ---- Write response headers (status + headers, no body). ----
    let (parts, mut response_body) = response.into_parts();
    let mut head = http::Response::new(());
    *head.status_mut() = parts.status;
    *head.headers_mut() = parts.headers;
    *head.version_mut() = http::Version::HTTP_3;
    {
        let mut guard = stream.lock().await;
        guard
            .send_response(head)
            .await
            .context("h3 send_response")?;
    }

    // ---- Drain body frames, sending data + trailers as we go. ----
    //
    // Streaming round-trip: each `Frame::data` from the response body becomes
    // exactly one `send_data` call on the h3 stream. Connect's server-streaming
    // RPC envelope (5-byte header + length-prefixed proto message per frame)
    // is produced by ConnectRpcService at the http_body layer; we forward
    // chunks faithfully without merging or splitting.
    let mut response_body = Pin::new(&mut response_body);
    loop {
        let frame = std::future::poll_fn(|cx| response_body.as_mut().poll_frame(cx)).await;
        match frame {
            Some(Ok(frame)) => {
                if frame.is_data() {
                    let data = frame.into_data().unwrap_or_else(|_| Bytes::new());
                    if !data.is_empty() {
                        let mut guard = stream.lock().await;
                        guard.send_data(data).await.context("h3 send_data")?;
                    }
                } else if frame.is_trailers() {
                    let trailers = match frame.into_trailers() {
                        Ok(t) => t,
                        Err(_) => continue,
                    };
                    let mut guard = stream.lock().await;
                    guard
                        .send_trailers(trailers)
                        .await
                        .context("h3 send_trailers")?;
                }
            }
            Some(Err(err)) => {
                tracing::warn!(?err, "response body error");
                break;
            }
            None => break,
        }
    }

    {
        let mut guard = stream.lock().await;
        guard.finish().await.context("h3 stream finish")?;
    }
    Ok(())
}
