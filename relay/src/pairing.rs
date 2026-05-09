//! In-memory pairing table for the pairing-mediated rendezvous pattern
//! (NFC / QR in-person handshake).
//!
//! Per spec: `(nonce, expires_at, attached_session_id_or_null)` plus a hash of
//! the per-pairing credential. Default TTL is 60 seconds for the *unattached*
//! lifetime — a pairing that has been attached gets bumped to 30-min idle TTL
//! by callers when they update the attachment.
//!
//! Expiry is enforced by:
//! 1. A background sweeper task (started by `PairingTable::start_sweeper`)
//!    that walks the table on every `tokio::time::sleep` tick.
//! 2. Lazy filtering inside `lookup`: an expired row is treated as absent
//!    even if the sweeper hasn't run yet.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tokio::time::Instant;

/// Default TTL for an unattached pairing nonce.
pub const DEFAULT_UNATTACHED_TTL: Duration = Duration::from_secs(60);

/// Bumped TTL once an operator client has attached.
pub const ATTACHED_IDLE_TTL: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone)]
pub struct PairingRow {
    pub nonce: String,
    pub expires_at: Instant,
    pub attached_session_id: Option<u32>,
    pub per_pairing_credential_hash: [u8; 32],
}

#[derive(Default)]
pub struct PairingTable {
    inner: Arc<RwLock<HashMap<String, PairingRow>>>,
}

impl PairingTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh pairing row with the default unattached TTL.
    pub async fn insert(&self, row: PairingRow) {
        let mut g = self.inner.write().await;
        g.insert(row.nonce.clone(), row);
    }

    /// Insert with explicit TTL.
    pub async fn insert_with_ttl(&self, mut row: PairingRow, ttl: Duration) {
        row.expires_at = Instant::now() + ttl;
        let mut g = self.inner.write().await;
        g.insert(row.nonce.clone(), row);
    }

    /// Look up a nonce. Returns `None` if absent or expired.
    pub async fn lookup(&self, nonce: &str) -> Option<PairingRow> {
        let g = self.inner.read().await;
        match g.get(nonce) {
            Some(row) if row.expires_at > Instant::now() => Some(row.clone()),
            _ => None,
        }
    }

    /// Mark a pairing as attached and extend its TTL.
    pub async fn attach(&self, nonce: &str, session_id: u32) -> bool {
        let mut g = self.inner.write().await;
        if let Some(row) = g.get_mut(nonce) {
            if row.expires_at <= Instant::now() {
                return false;
            }
            row.attached_session_id = Some(session_id);
            row.expires_at = Instant::now() + ATTACHED_IDLE_TTL;
            true
        } else {
            false
        }
    }

    /// Drop a pairing row (e.g. on session close).
    pub async fn remove(&self, nonce: &str) -> bool {
        let mut g = self.inner.write().await;
        g.remove(nonce).is_some()
    }

    /// Walk the table and drop any expired rows. Returns the number swept.
    pub async fn sweep_expired(&self) -> usize {
        let now = Instant::now();
        let mut g = self.inner.write().await;
        let before = g.len();
        g.retain(|_, row| row.expires_at > now);
        before - g.len()
    }

    /// Number of currently-tracked rows (including not-yet-swept expired ones).
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }

    /// Spawn a background task that calls `sweep_expired` every `interval`.
    /// The task lives as long as the returned `JoinHandle` (drop to stop).
    pub fn start_sweeper(self: &Arc<Self>, interval: Duration) -> tokio::task::JoinHandle<()> {
        let me = self.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await; // skip the immediate fire
            loop {
                ticker.tick().await;
                me.sweep_expired().await;
            }
        })
    }
}

impl PairingRow {
    /// Convenience constructor that sets the default unattached TTL from now.
    pub fn new(nonce: impl Into<String>, credential_hash: [u8; 32]) -> Self {
        Self {
            nonce: nonce.into(),
            expires_at: Instant::now() + DEFAULT_UNATTACHED_TTL,
            attached_session_id: None,
            per_pairing_credential_hash: credential_hash,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn insert_and_lookup() {
        let table = PairingTable::new();
        let row = PairingRow::new("nonce-1", [0u8; 32]);
        table.insert(row.clone()).await;
        let got = table.lookup("nonce-1").await.unwrap();
        assert_eq!(got.nonce, "nonce-1");
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn lookup_skips_expired() {
        let table = PairingTable::new();
        table
            .insert_with_ttl(
                PairingRow::new("expiring", [0u8; 32]),
                Duration::from_millis(50),
            )
            .await;
        // advance virtual clock past expiry
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(table.lookup("expiring").await.is_none());
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn attach_extends_ttl_and_records_session() {
        let table = PairingTable::new();
        table
            .insert_with_ttl(
                PairingRow::new("nonce-2", [0u8; 32]),
                Duration::from_millis(50),
            )
            .await;
        assert!(table.attach("nonce-2", 7).await);
        // beyond original 50ms, but well within the attached TTL
        tokio::time::sleep(Duration::from_millis(100)).await;
        let got = table.lookup("nonce-2").await.unwrap();
        assert_eq!(got.attached_session_id, Some(7));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn sweep_drops_expired_rows() {
        let table = PairingTable::new();
        table
            .insert_with_ttl(
                PairingRow::new("a", [0; 32]),
                Duration::from_millis(10),
            )
            .await;
        table.insert(PairingRow::new("b", [0; 32])).await;
        tokio::time::sleep(Duration::from_millis(20)).await;
        let swept = table.sweep_expired().await;
        assert_eq!(swept, 1);
        assert_eq!(table.len().await, 1);
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn remove() {
        let table = PairingTable::new();
        table.insert(PairingRow::new("x", [0; 32])).await;
        assert!(table.remove("x").await);
        assert!(table.lookup("x").await.is_none());
    }
}
