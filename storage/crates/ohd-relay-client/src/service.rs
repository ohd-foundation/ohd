//! Server-only session handler: bridge relay sessions onto a local
//! `connectrpc::ConnectRpcService` over hyper HTTP/2.
//!
//! Gated behind the `tunnel-service` feature — it pulls hyper / hyper-util
//! / connectrpc / tower, which are heavy and server-only. The Android
//! uniffi binding builds the crate without this feature and supplies its
//! own [`crate::tunnel::SessionHandler`].
//!
//! # Demuxing consumer sessions onto the local Connect-RPC service
//!
//! Each per-session bidi stream pair carries the consumer's HTTP/2
//! connection bytes wrapped in `DATA` frame envelopes. We bridge each pair
//! into an `AsyncRead + AsyncWrite` ([`SessionConn`]) that:
//!
//! - On `poll_read`: drains queued DATA-frame payloads. When the QUIC
//!   stream finishes (or a CLOSE arrives), `poll_read` returns 0.
//! - On `poll_write`: wraps the bytes into a single `DATA` envelope and
//!   writes to the QUIC `SendStream`. Larger writes are chunked at
//!   [`MAX_OUTBOUND_PAYLOAD_BYTES`].
//!
//! The bridged conn is handed to hyper's `auto::Builder` (HTTP/1 disabled,
//! HTTP/2 enabled); the hyper server calls into the `ConnectRpcService`.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use anyhow::{Context as _, Result};
use bytes::{Bytes, BytesMut};
use connectrpc::ConnectRpcService;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder as AutoBuilder;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::sync::{mpsc, Mutex};
use tracing::debug;

use crate::frame::{decode_one_frame, encode_frame, FrameError, FrameType};
use crate::tunnel::{AcceptedSession, SessionHandler};

/// Per-session inbound channel depth (DATA frames buffered before
/// backpressure kicks in).
pub const SESSION_INBOUND_BUFFER: usize = 256;

/// Cap on a single outbound `DATA` payload (the wire's u16 `payload_len`
/// field, minus headroom). Larger writes are chunked transparently.
pub const MAX_OUTBOUND_PAYLOAD_BYTES: usize = 60 * 1024;

/// A [`SessionHandler`] that bridges relay sessions into a local
/// `ConnectRpcService` over hyper HTTP/2.
#[derive(Clone)]
pub struct ConnectRpcSessionHandler {
    service: ConnectRpcService,
}

impl ConnectRpcSessionHandler {
    /// Wrap a Connect-RPC service for relay-tunnel session bridging.
    pub fn new(service: ConnectRpcService) -> Self {
        Self { service }
    }
}

impl SessionHandler for ConnectRpcSessionHandler {
    fn handle(
        &self,
        session: AcceptedSession,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + Send>> {
        let service = self.service.clone();
        Box::pin(async move { handle_session_stream(session, service).await })
    }
}

/// Bridge one accepted session into hyper HTTP/2: send OPEN_ACK, pump DATA
/// frames into a `SessionConn`, serve the local Connect-RPC service.
async fn handle_session_stream(
    session: AcceptedSession,
    service: ConnectRpcService,
) -> Result<()> {
    let AcceptedSession {
        session_id,
        mut send,
        recv,
        open_frame: _,
        leftover,
    } = session;

    // The relay's synthetic OPEN envelope was already read + verified by
    // `dispatch_session_stream`; reply with OPEN_ACK.
    let ack = encode_frame(FrameType::OpenAck, session_id, &[]);
    send.write_all(&ack).await.context("write OPEN_ACK")?;

    let (inbound_tx, inbound_rx) = mpsc::channel::<Bytes>(SESSION_INBOUND_BUFFER);
    let send_arc = Arc::new(Mutex::new(send));

    let send_for_reader = Arc::clone(&send_arc);
    let reader_task = tokio::spawn(async move {
        if let Err(err) =
            pump_inbound(session_id, recv, leftover, inbound_tx, send_for_reader).await
        {
            debug!(
                target: "ohd_relay_client::service",
                session_id, ?err, "inbound pump exited with error"
            );
        }
    });

    let conn_io = SessionConn::new(session_id, inbound_rx, Arc::clone(&send_arc));
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
        debug!(
            target: "ohd_relay_client::service",
            session_id, ?err, "hyper serve_connection ended"
        );
    }

    reader_task.abort();
    let close = encode_frame(FrameType::Close, session_id, &[]);
    {
        let mut s = send_arc.lock().await;
        let _ = s.write_all(&close).await;
        let _ = s.finish();
    }
    Ok(())
}

/// Pump frames off the QUIC RecvStream into the inbound mpsc.
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
                            debug!(
                                target: "ohd_relay_client::service",
                                session_id, "CLOSE from relay; ending session"
                            );
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
                            debug!(
                                target: "ohd_relay_client::service",
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

        let mut chunk = vec![0u8; 16 * 1024];
        match recv.read(&mut chunk).await? {
            Some(n) => seed.extend_from_slice(&chunk[..n]),
            None => return Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// SessionConn: AsyncRead + AsyncWrite over a DATA-frame envelope pair
// ---------------------------------------------------------------------------

/// Adapter that bridges a relay session into a `tokio::io::{AsyncRead,
/// AsyncWrite}` pair so hyper can `serve_connection()` over it.
struct SessionConn {
    session_id: u32,
    inbound: mpsc::Receiver<Bytes>,
    leftover: Bytes,
    send: Arc<Mutex<quinn::SendStream>>,
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
        if !self.leftover.is_empty() {
            let take = std::cmp::min(self.leftover.len(), out.remaining());
            let chunk = self.leftover.split_to(take);
            out.put_slice(&chunk);
            return Poll::Ready(Ok(()));
        }
        match self.inbound.poll_recv(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(None) => Poll::Ready(Ok(())),
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
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
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
        Poll::Ready(Ok(()))
    }
}

fn io_err<E: std::fmt::Display>(err: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err.to_string())
}

/// Run the outbound relay tunnel client, bridging accepted sessions onto
/// `service`. Convenience wrapper around [`crate::tunnel::serve_relay_tunnel`]
/// + [`ConnectRpcSessionHandler`] — preserves the pre-extraction
/// `serve_relay_tunnel(opts, service, shutdown)` call shape that
/// `ohd-storage-server` used.
pub async fn serve_relay_tunnel(
    opts: crate::tunnel::RelayClientOptions,
    service: ConnectRpcService,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<()> {
    let handler: Arc<dyn SessionHandler> =
        Arc::new(ConnectRpcSessionHandler::new(service));
    crate::tunnel::serve_relay_tunnel(opts, handler, shutdown).await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    // The pump_inbound / SessionConn paths need a live quinn stream pair to
    // exercise meaningfully; those are covered by ohd-storage-server's
    // integration tests. Here we keep a lightweight sanity check on the
    // chunking constant only.
    use super::*;

    #[test]
    fn outbound_chunk_cap_within_frame_limit() {
        assert!(MAX_OUTBOUND_PAYLOAD_BYTES <= crate::frame::MAX_PAYLOAD_LEN);
    }

    #[test]
    fn data_frame_encodes_with_session_id() {
        let payload = vec![0xAB; MAX_OUTBOUND_PAYLOAD_BYTES];
        let bytes = encode_frame(FrameType::Data, 5, &payload);
        let (frame, _) = decode_one_frame(&bytes).unwrap();
        assert_eq!(frame.frame_type, FrameType::Data);
        assert_eq!(frame.session_id, 5);
    }
}
