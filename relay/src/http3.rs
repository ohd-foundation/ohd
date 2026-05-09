//! HTTP/3 (QUIC) listener for the relay's REST endpoints.
//!
//! # Scope
//!
//! Mounts the same `axum::Router` the HTTP/2 listener uses (built by
//! [`crate::server::build_router`]) on top of `quinn` + `h3` so the REST
//! endpoints — `POST /v1/register`, `POST /v1/heartbeat`, `POST
//! /v1/deregister`, `GET /health` — are reachable over HTTP/3 by mobile +
//! cellular clients that prefer QUIC.
//!
//! ## What's NOT here: WebSocket-over-HTTP/3
//!
//! RFC 9220 (Bootstrapping WebSockets with HTTP/3) requires extended
//! CONNECT support that the `h3 0.0.x` crate does not yet expose to axum
//! cleanly. The relay's tunnel + attach paths (`WS /v1/tunnel/:id` and `WS
//! /v1/attach/:id`) therefore stay on the HTTP/2 listener; clients that
//! need the persistent bidi tunnel keep dialling that one.
//!
//! Operationally this means the HTTP/3 path is lower-overhead for control
//! plane (push registration / heartbeat) but the data plane (relay tunnels)
//! still uses HTTP/2 over TLS. Once `h3` ships extended-CONNECT we can lift
//! the WS routes onto HTTP/3 too without changing the axum router shape.
//!
//! ## Why in-binary, not Caddy-fronted
//!
//! Same reasoning as the storage server (single static binary, lower
//! socket count under high event load, mobile-friendly QUIC migration).
//! See `storage/STATUS.md` "HTTP/3 (in-binary) — landed" for the long
//! version.
//!
//! ## Shared primitives
//!
//! Cert loading (PEM-from-file + dev self-signed via rcgen), the QUIC
//! `ServerConfig` setup, and the [`H3RequestBody`] streaming adapter all
//! live in `ohd-h3-helpers` (in the storage workspace) — same crate the
//! storage server uses. Only the per-request dispatch glue (calling into
//! the axum `Router<()>` and pumping its response body back through the
//! h3 stream) stays here. That glue can't quite share with storage's
//! variant: storage hands the request body to `ConnectRpcService` raw,
//! while this binary wraps it in `axum::body::Body::new(...)` first.
//!
//! ## Production cert flags
//!
//! [`load_pem_cert_key`] reads a PEM cert chain + private key from disk
//! using `rustls-pemfile 2`. The `serve` subcommand wires
//! `--http3-cert PATH` / `--http3-key PATH` through to this helper; absent
//! both flags, [`dev_self_signed_cert`] is used and a stderr warning is
//! emitted.

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use anyhow::Context as _;
use axum::body::Body as AxumBody;
use axum::Router;
use bytes::Bytes;
use http::Request;
use http_body::Body;
use tokio::sync::Mutex;
use tower::Service;

// Re-export the shared helpers under their original names so the rest of
// the binary can keep saying `crate::http3::load_pem_cert_key` etc.
// `server_config`, `H3BodyError`, and the body-builder helpers are part
// of the public surface even when this file only uses some directly.
#[allow(unused_imports)]
pub use ohd_h3_helpers::{
    convert_recv_chunk, dev_self_signed_cert, load_pem_cert_key, make_endpoint, server_config,
    BoxedRecvFuture, H3BodyError, H3RequestBody,
};

/// Run the relay HTTP/3 (QUIC) listener until cancelled.
///
/// The listener accepts QUIC connections and dispatches each accepted h3
/// request through the supplied [`Router<()>`] (which must be the
/// fully-stated router, i.e. `with_state` already applied — relay's
/// `build_router` returns one).
pub async fn serve(
    addr: SocketAddr,
    router: Router<()>,
    cert_chain: Vec<rustls::pki_types::CertificateDer<'static>>,
    key: rustls::pki_types::PrivateKeyDer<'static>,
) -> anyhow::Result<()> {
    let endpoint = make_endpoint(addr, cert_chain, key)?;
    tracing::info!(target: "ohd_relay::http3", %addr, "ohd-relay HTTP/3 listening");

    while let Some(incoming) = endpoint.accept().await {
        let router = router.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(incoming, router).await {
                tracing::warn!(target: "ohd_relay::http3", ?err, "h3 connection ended");
            }
        });
    }
    Ok(())
}

async fn handle_connection(
    incoming: quinn::Incoming,
    router: Router<()>,
) -> anyhow::Result<()> {
    let conn = incoming.await.context("quinn handshake")?;
    let mut h3_conn = h3::server::Connection::<_, Bytes>::new(h3_quinn::Connection::new(conn))
        .await
        .context("h3 server connection")?;

    loop {
        match h3_conn.accept().await {
            Ok(Some(resolver)) => {
                let router = router.clone();
                tokio::spawn(async move {
                    if let Err(err) = handle_request(resolver, router).await {
                        tracing::warn!(target: "ohd_relay::http3", ?err, "h3 request error");
                    }
                });
            }
            Ok(None) => break,
            Err(err) => {
                tracing::debug!(target: "ohd_relay::http3", ?err, "h3 accept error");
                break;
            }
        }
    }
    Ok(())
}

async fn handle_request<C>(
    resolver: h3::server::RequestResolver<C, Bytes>,
    mut router: Router<()>,
) -> anyhow::Result<()>
where
    C: h3::quic::Connection<Bytes>,
    <C as h3::quic::OpenStreams<Bytes>>::BidiStream: Send + 'static,
{
    let (req, stream) = resolver.resolve_request().await.context("resolve_request")?;

    let stream = Arc::new(Mutex::new(stream));

    // Build a streaming body. `AxumBody::new` accepts any `Body<Data=Bytes>`
    // with a compatible error type, so we wrap our adapter in axum's body
    // type to feed the router unchanged.
    let body_stream = Arc::clone(&stream);
    let h3_body = H3RequestBody::new(move || -> BoxedRecvFuture {
        let s = Arc::clone(&body_stream);
        Box::pin(async move {
            let mut guard = s.lock().await;
            convert_recv_chunk(guard.recv_data().await)
        })
    });
    let body = AxumBody::new(h3_body);

    // Rebuild the request with our streaming body.
    let (parts, ()) = req.into_parts();
    let request: Request<AxumBody> = Request::from_parts(parts, body);

    // Dispatch through the axum Router. Router<()>::Error is Infallible.
    let response = router
        .call(request)
        .await
        .unwrap_or_else(|never| match never {});

    // Send headers, then drain body frames out via h3.
    let (parts, mut response_body) = response.into_parts();
    let mut head = http::Response::new(());
    *head.status_mut() = parts.status;
    *head.headers_mut() = parts.headers;
    *head.version_mut() = http::Version::HTTP_3;
    {
        let mut guard = stream.lock().await;
        guard.send_response(head).await.context("send_response")?;
    }

    let mut response_body = Pin::new(&mut response_body);
    loop {
        let frame = std::future::poll_fn(|cx| response_body.as_mut().poll_frame(cx)).await;
        match frame {
            Some(Ok(frame)) => {
                if frame.is_data() {
                    let data = frame.into_data().unwrap_or_else(|_| Bytes::new());
                    if !data.is_empty() {
                        let mut guard = stream.lock().await;
                        guard.send_data(data).await.context("send_data")?;
                    }
                } else if frame.is_trailers() {
                    let trailers = match frame.into_trailers() {
                        Ok(t) => t,
                        Err(_) => continue,
                    };
                    let mut guard = stream.lock().await;
                    guard.send_trailers(trailers).await.context("send_trailers")?;
                }
            }
            Some(Err(err)) => {
                tracing::warn!(target: "ohd_relay::http3", ?err, "response body error");
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
