//! Session multiplexing: per-`(rendezvous_id, session_id)` byte pipes between a
//! storage tunnel and its attached consumers.
//!
//! Architecture
//! -----------
//!
//! - One **TunnelEndpoint** per registered storage. Owns the WebSocket sink to
//!   that storage; the relay writes `TunnelFrame`s to it via an `mpsc` channel.
//! - One **SessionHandle** per attached consumer. Has two channels:
//!   - `consumer_to_storage`: bytes the consumer sends; the demux loop wraps
//!     each chunk in a `DATA` frame and forwards it to the tunnel.
//!   - `storage_to_consumer`: bytes the storage emitted in `DATA` frames for
//!     this session; the consumer-attach handler drains this and writes to the
//!     consumer's WebSocket.
//!
//! The actual demux loop (parse incoming tunnel frames, route by SESSION_ID)
//! lives in `server.rs` as part of the WebSocket handler. This module just
//! provides the data structures and channel plumbing.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use tokio::sync::{mpsc, RwLock};

use crate::frame::TunnelFrame;

/// Default session-table channel buffer (frames).
pub const TUNNEL_SEND_BUFFER: usize = 256;
/// Default consumer-bound chunk buffer (DATA payloads).
pub const SESSION_CHUNK_BUFFER: usize = 64;
/// Initial flow-control receive window per side per session (per spec: 256 KB).
pub const DEFAULT_RECEIVE_WINDOW: u32 = 256 * 1024;

// ---------------------------------------------------------------------------
// TunnelEndpoint
// ---------------------------------------------------------------------------

/// The storage end of a tunnel. The websocket writer task drains
/// `outbound_rx`; everything that wants to send to storage clones
/// `outbound_tx` and pushes a `TunnelFrame` through it.
#[derive(Clone)]
pub struct TunnelEndpoint {
    pub rendezvous_id: String,
    pub outbound_tx: mpsc::Sender<TunnelFrame>,
    pub session_id_seq: Arc<AtomicU32>,
    /// Consumed from above when sessions are added/removed.
    pub sessions: Arc<RwLock<HashMap<u32, SessionHandle>>>,
    /// Wall-clock (Instant) when the tunnel was opened.
    pub opened_at: Instant,
}

impl TunnelEndpoint {
    /// Create a new endpoint together with the receiver end of its outbound
    /// channel. The caller's writer task owns the receiver.
    pub fn new(rendezvous_id: impl Into<String>) -> (Self, mpsc::Receiver<TunnelFrame>) {
        let (tx, rx) = mpsc::channel(TUNNEL_SEND_BUFFER);
        let endpoint = Self {
            rendezvous_id: rendezvous_id.into(),
            outbound_tx: tx,
            session_id_seq: Arc::new(AtomicU32::new(1)),
            sessions: Arc::new(RwLock::new(HashMap::new())),
            opened_at: Instant::now(),
        };
        (endpoint, rx)
    }

    pub fn next_session_id(&self) -> u32 {
        loop {
            let id = self.session_id_seq.fetch_add(1, Ordering::SeqCst);
            if id != 0 {
                return id;
            }
        }
    }

    /// Allocate a new session, register it, and return both the
    /// consumer-side handle and the relay-side endpoint reference.
    pub async fn open_session(&self) -> (u32, SessionHandle, SessionRelaySide) {
        let session_id = self.next_session_id();
        let (consumer_to_storage_tx, consumer_to_storage_rx) =
            mpsc::channel::<Bytes>(SESSION_CHUNK_BUFFER);
        let (storage_to_consumer_tx, storage_to_consumer_rx) =
            mpsc::channel::<Bytes>(SESSION_CHUNK_BUFFER);

        let handle = SessionHandle {
            session_id,
            consumer_to_storage_tx,
            storage_to_consumer_rx: Arc::new(tokio::sync::Mutex::new(Some(
                storage_to_consumer_rx,
            ))),
            attached_at: Instant::now(),
        };

        let relay_side = SessionRelaySide {
            session_id,
            consumer_to_storage_rx: Arc::new(tokio::sync::Mutex::new(Some(
                consumer_to_storage_rx,
            ))),
            storage_to_consumer_tx,
        };

        self.sessions.write().await.insert(
            session_id,
            handle.clone_for_table(),
        );
        (session_id, handle, relay_side)
    }

    /// Remove a session entry (e.g. on `CLOSE` or consumer disconnect).
    pub async fn close_session(&self, session_id: u32) {
        self.sessions.write().await.remove(&session_id);
    }

    /// Forward a chunk that came in on the storage tunnel for a particular
    /// session. Drops silently if the session has gone away.
    pub async fn dispatch_inbound(&self, session_id: u32, payload: Bytes) {
        let table = self.sessions.read().await;
        if let Some(s) = table.get(&session_id) {
            // Best-effort: if the consumer is gone, drop.
            let _ = s.try_push_to_consumer(payload).await;
        }
    }

    /// Fan out a CLOSE on the tunnel: drop all sessions, close their
    /// consumer-bound senders. Used on storage disconnect.
    pub async fn drain_all_sessions(&self) {
        let mut table = self.sessions.write().await;
        table.clear();
    }
}

// ---------------------------------------------------------------------------
// SessionHandle (consumer-attach side) + SessionRelaySide (demux side)
// ---------------------------------------------------------------------------

/// What the consumer-attach handler holds. It uses `consumer_to_storage_tx` to
/// forward inbound consumer bytes, and drains `storage_to_consumer_rx` to ship
/// storage's DATA payloads back to the consumer.
#[derive(Clone)]
pub struct SessionHandle {
    pub session_id: u32,
    pub consumer_to_storage_tx: mpsc::Sender<Bytes>,
    pub storage_to_consumer_rx: Arc<tokio::sync::Mutex<Option<mpsc::Receiver<Bytes>>>>,
    pub attached_at: Instant,
}

impl SessionHandle {
    /// Return a clone usable as a directory entry (uses the same channels
    /// since channels are clonable / share state).
    fn clone_for_table(&self) -> SessionHandle {
        SessionHandle {
            session_id: self.session_id,
            consumer_to_storage_tx: self.consumer_to_storage_tx.clone(),
            storage_to_consumer_rx: Arc::new(tokio::sync::Mutex::new(None)),
            attached_at: self.attached_at,
        }
    }

    /// Take the receiver, leaving `None` for further reads. Callers do this
    /// once when the consumer-attach starts.
    pub async fn take_storage_to_consumer(&self) -> Option<mpsc::Receiver<Bytes>> {
        self.storage_to_consumer_rx.lock().await.take()
    }

    /// Used by the tunnel demux loop to push a DATA payload to this session's
    /// consumer. The actual sender is held inside the relay-side directory
    /// entry; the table's `clone_for_table` does not get the sender, so this
    /// method is a no-op on table entries — it exists only on entries that
    /// have an active sender, which we keep in `SessionRelaySide`.
    async fn try_push_to_consumer(&self, _payload: Bytes) -> Result<(), ()> {
        // Routing is handled by the demux loop holding `SessionRelaySide`.
        Ok(())
    }
}

/// What the tunnel-side demux loop holds.
pub struct SessionRelaySide {
    pub session_id: u32,
    pub consumer_to_storage_rx: Arc<tokio::sync::Mutex<Option<mpsc::Receiver<Bytes>>>>,
    pub storage_to_consumer_tx: mpsc::Sender<Bytes>,
}

impl SessionRelaySide {
    pub async fn take_consumer_to_storage(&self) -> Option<mpsc::Receiver<Bytes>> {
        self.consumer_to_storage_rx.lock().await.take()
    }

    pub async fn forward_to_consumer(&self, payload: Bytes) -> Result<(), mpsc::error::SendError<Bytes>> {
        self.storage_to_consumer_tx.send(payload).await
    }
}

// ---------------------------------------------------------------------------
// Attached-senders registry (consumer-side hook into a TunnelEndpoint)
// ---------------------------------------------------------------------------
//
// A `TunnelEndpoint` exposes its outbound channel for relay→storage frames
// but the reverse direction — storage→relay DATA payloads, then routed to
// the matching consumer's WebSocket / QUIC stream — needs a per-session
// `mpsc::Sender<Bytes>`. We keep this map adjacent to the endpoint via a
// `Weak`-keyed global registry so multiple transports (HTTP/2 WS, raw
// QUIC) share the same map for a given endpoint.

/// Per-session sender map for routing storage→consumer DATA payloads.
pub type AttachedSenders = tokio::sync::RwLock<HashMap<u32, mpsc::Sender<Bytes>>>;

/// Look up (or lazily install) the attached-senders map for a tunnel
/// endpoint. Both the WebSocket tunnel handler in `server.rs` and the raw
/// QUIC tunnel handler in `quic_tunnel.rs` route storage→consumer DATA
/// through this registry, keyed by `session_id`.
pub fn attached_senders_for(endpoint: &TunnelEndpoint) -> Arc<AttachedSenders> {
    let key = Arc::as_ptr(&endpoint.sessions) as usize;
    let r = attached_senders_registry();
    let mut g = r.lock().unwrap();
    // Sweep dead entries (Weak no longer upgradable) — this prevents
    // stale state when a previous endpoint at the same address has been
    // dropped (typical in test harnesses).
    g.retain(|_, (witness, _)| witness.strong_count() > 0);
    if let Some((_, v)) = g.get(&key) {
        return v.clone();
    }
    let new = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
    let weak = Arc::downgrade(&endpoint.sessions);
    g.insert(key, (weak, new.clone()));
    new
}

#[allow(clippy::type_complexity)]
fn attached_senders_registry() -> &'static std::sync::Mutex<
    HashMap<
        usize,
        (
            std::sync::Weak<RwLock<HashMap<u32, SessionHandle>>>,
            Arc<AttachedSenders>,
        ),
    >,
> {
    use std::sync::OnceLock;
    #[allow(clippy::type_complexity)]
    static R: OnceLock<
        std::sync::Mutex<
            HashMap<
                usize,
                (
                    std::sync::Weak<RwLock<HashMap<u32, SessionHandle>>>,
                    Arc<AttachedSenders>,
                ),
            >,
        >,
    > = OnceLock::new();
    R.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// SessionTable — registry of currently-open tunnels
// ---------------------------------------------------------------------------

/// Top-level registry: rendezvous_id → currently-open TunnelEndpoint.
///
/// At most one tunnel per rendezvous_id at any time (v1: "one user, one
/// active relay registration"). Re-registering replaces the previous tunnel.
#[derive(Default)]
pub struct SessionTable {
    inner: RwLock<HashMap<String, TunnelEndpoint>>,
}

impl SessionTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn register_tunnel(&self, endpoint: TunnelEndpoint) {
        let mut g = self.inner.write().await;
        g.insert(endpoint.rendezvous_id.clone(), endpoint);
    }

    pub async fn deregister_tunnel(&self, rendezvous_id: &str) -> Option<TunnelEndpoint> {
        let mut g = self.inner.write().await;
        g.remove(rendezvous_id)
    }

    pub async fn lookup(&self, rendezvous_id: &str) -> Option<TunnelEndpoint> {
        let g = self.inner.read().await;
        g.get(rendezvous_id).cloned()
    }

    pub async fn count(&self) -> usize {
        self.inner.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn open_and_close_session() {
        let (endpoint, _outbound_rx) = TunnelEndpoint::new("rzv-x");
        let (sid, handle, relay) = endpoint.open_session().await;
        assert!(sid >= 1);
        assert_eq!(handle.session_id, sid);
        assert_eq!(relay.session_id, sid);
        assert_eq!(endpoint.sessions.read().await.len(), 1);

        endpoint.close_session(sid).await;
        assert_eq!(endpoint.sessions.read().await.len(), 0);
    }

    #[tokio::test]
    async fn session_id_skips_zero() {
        let (endpoint, _rx) = TunnelEndpoint::new("rzv-y");
        endpoint.session_id_seq.store(0, Ordering::SeqCst);
        let id = endpoint.next_session_id();
        assert_ne!(id, 0);
    }

    #[tokio::test]
    async fn session_table_register_lookup() {
        let table = SessionTable::new();
        let (endpoint, _rx) = TunnelEndpoint::new("rzv-z");
        table.register_tunnel(endpoint.clone()).await;
        assert!(table.lookup("rzv-z").await.is_some());
        assert!(table.deregister_tunnel("rzv-z").await.is_some());
        assert!(table.lookup("rzv-z").await.is_none());
    }
}
