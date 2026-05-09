//! Relay state: the SQLite-backed `RegistrationTable` plus references to the
//! in-memory `SessionTable` and `PairingTable` (defined in their own modules).
//!
//! Per the spec's "five-field per-user state":
//!
//! ```text
//! (rendezvous_id, user_ulid, current_tunnel_endpoint, push_token, last_heartbeat_at)
//! ```
//!
//! plus a small log of recent connection events for operational telemetry.
//! **Nothing else** — no grants, no audit, no tokens, no PHI. The relay's
//! privacy property — "compromise reveals only traffic patterns" — depends on
//! this minimalism.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rusqlite::{params, Connection};
use tokio::sync::Mutex;

use crate::pairing::PairingTable;
use crate::session::SessionTable;

// ---------------------------------------------------------------------------
// Top-level state handle
// ---------------------------------------------------------------------------

/// Bundles the three core tables. Cloned cheaply (`Arc` interior) and shared
/// across all request handlers.
#[derive(Clone)]
pub struct RelayState {
    pub registrations: Arc<RegistrationTable>,
    pub sessions: Arc<SessionTable>,
    pub pairings: Arc<PairingTable>,
}

impl RelayState {
    /// Open (or create) the relay state with the given on-disk SQLite path.
    pub async fn open(db_path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let registrations = RegistrationTable::open(db_path).await?;
        Ok(Self {
            registrations: Arc::new(registrations),
            sessions: Arc::new(SessionTable::new()),
            pairings: Arc::new(PairingTable::new()),
        })
    }

    /// Convenience: in-memory SQLite, useful for tests.
    pub async fn in_memory() -> anyhow::Result<Self> {
        let registrations = RegistrationTable::in_memory().await?;
        Ok(Self {
            registrations: Arc::new(registrations),
            sessions: Arc::new(SessionTable::new()),
            pairings: Arc::new(PairingTable::new()),
        })
    }
}

// ---------------------------------------------------------------------------
// Registration row + supporting types
// ---------------------------------------------------------------------------

/// Per-user registration row. Persisted in SQLite.
///
/// Constraints:
/// - `UNIQUE(rendezvous_id)`.
/// - One active registration per `(relay, user_ulid)` in v1.
/// - GC after 30 days of no successful tunnel-open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistrationRow {
    pub rendezvous_id: String,
    pub user_ulid: [u8; 16],
    pub push_token: Option<PushToken>,
    pub last_heartbeat_at_ms: i64,
    pub long_lived_credential_hash: [u8; 32],
    pub registered_at_ms: i64,
    pub user_label: Option<String>,
    pub storage_pubkey: Vec<u8>,
    /// Optional OIDC issuer URL, set when this registration was gated by
    /// an `id_token` (per `[auth.registration]` in `relay.toml`).
    /// `None` when the relay is permissive or the operator declined to
    /// present a token. Recorded for audit only — not used for runtime
    /// auth decisions on subsequent calls (the `long_lived_credential`
    /// hash is the auth check from registration onward).
    pub oidc_iss: Option<String>,
    /// Optional OIDC subject identifier within the issuer. Pairs with
    /// `oidc_iss`. Same audit-only semantics.
    pub oidc_sub: Option<String>,
}

/// Push token bound to this registration's storage device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushToken {
    Fcm(String),
    Apns(String),
    WebPush {
        endpoint: String,
        p256dh: String,
        auth: String,
    },
    Email(String),
}

impl PushToken {
    pub fn platform(&self) -> &'static str {
        match self {
            Self::Fcm(_) => "fcm",
            Self::Apns(_) => "apns",
            Self::WebPush { .. } => "web_push",
            Self::Email(_) => "email",
        }
    }

    /// Encode as a URL-ish string for storage. Format:
    /// `fcm:<token>` / `apns:<token>` / `email:<addr>` /
    /// `webpush:<endpoint>|<p256dh>|<auth>`. Never round-trips through
    /// untrusted input — only the relay produces these strings.
    pub fn encode(&self) -> String {
        match self {
            Self::Fcm(t) => format!("fcm:{t}"),
            Self::Apns(t) => format!("apns:{t}"),
            Self::Email(a) => format!("email:{a}"),
            Self::WebPush {
                endpoint,
                p256dh,
                auth,
            } => format!("webpush:{endpoint}|{p256dh}|{auth}"),
        }
    }

    pub fn decode(s: &str) -> Option<Self> {
        let (prefix, rest) = s.split_once(':')?;
        match prefix {
            "fcm" => Some(Self::Fcm(rest.to_string())),
            "apns" => Some(Self::Apns(rest.to_string())),
            "email" => Some(Self::Email(rest.to_string())),
            "webpush" => {
                let mut parts = rest.splitn(3, '|');
                let endpoint = parts.next()?.to_string();
                let p256dh = parts.next()?.to_string();
                let auth = parts.next()?.to_string();
                Some(Self::WebPush {
                    endpoint,
                    p256dh,
                    auth,
                })
            }
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// RegistrationTable — SQLite-backed
// ---------------------------------------------------------------------------

/// Connection table. The rusqlite handle is sync; we wrap it in a mutex and
/// dispatch every operation through `tokio::task::spawn_blocking`.
pub struct RegistrationTable {
    /// `rusqlite::Connection` is `!Sync`, so we serialize access here. For v1
    /// single-instance throughput this is plenty; if it becomes a bottleneck,
    /// switch to a connection pool.
    conn: Arc<Mutex<Connection>>,
    /// Stored for diagnostics / reopen logic.
    path: PathBuf,
}

impl RegistrationTable {
    /// Open the persistent registration store at the given path. Creates the
    /// schema if not already present.
    pub async fn open(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let conn_path = path.clone();
        let conn = tokio::task::spawn_blocking(move || -> anyhow::Result<Connection> {
            let mut c = Connection::open(&conn_path)?;
            init_schema(&mut c)?;
            Ok(c)
        })
        .await??;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path,
        })
    }

    /// In-memory store for tests.
    pub async fn in_memory() -> anyhow::Result<Self> {
        let conn = tokio::task::spawn_blocking(|| -> anyhow::Result<Connection> {
            let mut c = Connection::open_in_memory()?;
            init_schema(&mut c)?;
            Ok(c)
        })
        .await??;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
            path: PathBuf::from(":memory:"),
        })
    }

    pub fn db_path(&self) -> &Path {
        &self.path
    }

    /// Hand out a clone of the inner `Connection` mutex so the
    /// `_emergency_requests` / `_emergency_handoffs` table accessors in
    /// `emergency_endpoints.rs` can serialize through the same lock.
    /// Single-instance v1 runs one SQLite connection for all writes; we
    /// don't want emergency CRUD racing the registration writer on a
    /// separate handle.
    pub fn conn_for_emergency(&self) -> Arc<Mutex<Connection>> {
        self.conn.clone()
    }

    /// Insert a new registration row. Errors on `UNIQUE(rendezvous_id)` /
    /// per-user uniqueness conflicts.
    pub async fn register(&self, row: RegistrationRow) -> anyhow::Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO registrations (
                    rendezvous_id, user_ulid, push_token, last_heartbeat_at_ms,
                    long_lived_credential_hash, registered_at_ms, user_label, storage_pubkey,
                    oidc_iss, oidc_sub
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    row.rendezvous_id,
                    row.user_ulid.to_vec(),
                    row.push_token.as_ref().map(|t| t.encode()),
                    row.last_heartbeat_at_ms,
                    row.long_lived_credential_hash.to_vec(),
                    row.registered_at_ms,
                    row.user_label,
                    row.storage_pubkey,
                    row.oidc_iss,
                    row.oidc_sub,
                ],
            )?;
            log_event(&conn, &row.rendezvous_id, "register", None)?;
            Ok(())
        })
        .await?
    }

    pub async fn lookup_by_rendezvous(
        &self,
        rendezvous_id: &str,
    ) -> anyhow::Result<Option<RegistrationRow>> {
        let conn = self.conn.clone();
        let id = rendezvous_id.to_string();
        tokio::task::spawn_blocking(move || -> anyhow::Result<Option<RegistrationRow>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT rendezvous_id, user_ulid, push_token, last_heartbeat_at_ms,
                        long_lived_credential_hash, registered_at_ms, user_label, storage_pubkey,
                        oidc_iss, oidc_sub
                 FROM registrations WHERE rendezvous_id = ?1",
            )?;
            let mut rows = stmt.query(params![id])?;
            if let Some(r) = rows.next()? {
                Ok(Some(row_from_sql(r)?))
            } else {
                Ok(None)
            }
        })
        .await?
    }

    /// Update a registration's `last_heartbeat_at_ms` and append a heartbeat
    /// event to the recent-events log. Returns whether the row was found.
    pub async fn heartbeat(
        &self,
        rendezvous_id: &str,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let id = rendezvous_id.to_string();
        tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
            let conn = conn.blocking_lock();
            let updated = conn.execute(
                "UPDATE registrations SET last_heartbeat_at_ms = ?1 WHERE rendezvous_id = ?2",
                params![now_ms, id],
            )?;
            if updated > 0 {
                log_event(&conn, &id, "heartbeat", None)?;
            }
            Ok(updated > 0)
        })
        .await?
    }

    /// Update the push token for a registration.
    pub async fn update_push_token(
        &self,
        rendezvous_id: &str,
        push_token: Option<PushToken>,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let id = rendezvous_id.to_string();
        let encoded = push_token.as_ref().map(|t| t.encode());
        tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
            let conn = conn.blocking_lock();
            let updated = conn.execute(
                "UPDATE registrations SET push_token = ?1 WHERE rendezvous_id = ?2",
                params![encoded, id],
            )?;
            if updated > 0 {
                log_event(&conn, &id, "update_push_token", None)?;
            }
            Ok(updated > 0)
        })
        .await?
    }

    /// Update the current tunnel endpoint marker. The endpoint itself lives
    /// in memory (it's a channel sender, not a persistable handle); this
    /// records that *some* tunnel is live for telemetry / push-wake
    /// decisions.
    pub async fn update_endpoint(
        &self,
        rendezvous_id: &str,
        connected: bool,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let id = rendezvous_id.to_string();
        let event = if connected {
            "tunnel_open"
        } else {
            "tunnel_close"
        };
        tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
            let conn = conn.blocking_lock();
            let updated = conn.execute(
                "UPDATE registrations SET last_heartbeat_at_ms = ?1 WHERE rendezvous_id = ?2",
                params![now_ms, id],
            )?;
            if updated > 0 {
                log_event(&conn, &id, event, None)?;
            }
            Ok(updated > 0)
        })
        .await?
    }

    /// Remove a registration entirely.
    pub async fn deregister(&self, rendezvous_id: &str) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        let id = rendezvous_id.to_string();
        tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
            let conn = conn.blocking_lock();
            log_event(&conn, &id, "deregister", None)?;
            let removed = conn.execute(
                "DELETE FROM registrations WHERE rendezvous_id = ?1",
                params![id],
            )?;
            Ok(removed > 0)
        })
        .await?
    }

    /// Count rows. Useful for tests and operator metrics.
    pub async fn count(&self) -> anyhow::Result<i64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
            let conn = conn.blocking_lock();
            let n: i64 = conn.query_row("SELECT COUNT(*) FROM registrations", [], |r| r.get(0))?;
            Ok(n)
        })
        .await?
    }

    /// Get the most recent N events for operator telemetry. Newest first.
    pub async fn recent_events(&self, limit: i64) -> anyhow::Result<Vec<RecentEvent>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<RecentEvent>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT id, rendezvous_id, kind, detail, at_ms
                 FROM registration_events
                 ORDER BY id DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], |r| {
                Ok(RecentEvent {
                    id: r.get(0)?,
                    rendezvous_id: r.get(1)?,
                    kind: r.get(2)?,
                    detail: r.get(3)?,
                    at_ms: r.get(4)?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })
        .await?
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentEvent {
    pub id: i64,
    pub rendezvous_id: String,
    pub kind: String,
    pub detail: Option<String>,
    pub at_ms: i64,
}

fn init_schema(conn: &mut Connection) -> anyhow::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS registrations (
            rendezvous_id              TEXT PRIMARY KEY,
            user_ulid                  BLOB NOT NULL,
            push_token                 TEXT,
            last_heartbeat_at_ms       INTEGER NOT NULL,
            long_lived_credential_hash BLOB NOT NULL,
            registered_at_ms           INTEGER NOT NULL,
            user_label                 TEXT,
            storage_pubkey             BLOB NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_registrations_user_ulid
            ON registrations (user_ulid);

        CREATE TABLE IF NOT EXISTS registration_events (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            rendezvous_id   TEXT NOT NULL,
            kind            TEXT NOT NULL,
            detail          TEXT,
            at_ms           INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_registration_events_at
            ON registration_events (at_ms DESC);

        -- Migration 002: emergency request bookkeeping. The relay tracks
        -- the in-flight state of break-glass requests between
        -- `/v1/emergency/initiate` and the patient phone's response so
        -- the operator's tablet can poll across socket disruptions. See
        -- `src/emergency_endpoints.rs` for the state machine + handlers.
        CREATE TABLE IF NOT EXISTS _emergency_requests (
            request_id        TEXT PRIMARY KEY,
            rendezvous_id     TEXT NOT NULL,
            state             TEXT NOT NULL,
            patient_label     TEXT,
            grant_token       TEXT,
            case_ulid         TEXT,
            rejected_reason   TEXT,
            default_action    TEXT,
            created_at_ms     INTEGER NOT NULL,
            decided_at_ms     INTEGER,
            expires_at_ms     INTEGER NOT NULL,
            gc_after_ms       INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_emergency_requests_rendezvous
            ON _emergency_requests (rendezvous_id);
        CREATE INDEX IF NOT EXISTS idx_emergency_requests_expires
            ON _emergency_requests (expires_at_ms);
        CREATE INDEX IF NOT EXISTS idx_emergency_requests_gc
            ON _emergency_requests (gc_after_ms);

        -- Operator-side handoff audit. Records the "who handed off to whom"
        -- view of a break-glass case transition; the actual case-state
        -- mutation (open successor + freeze predecessor) is the patient
        -- storage's job (see `OhdcService.HandoffCase`).
        CREATE TABLE IF NOT EXISTS _emergency_handoffs (
            audit_entry_ulid          TEXT PRIMARY KEY,
            source_case_ulid          TEXT NOT NULL,
            successor_case_ulid       TEXT NOT NULL,
            target_operator           TEXT NOT NULL,
            successor_operator_label  TEXT NOT NULL,
            handoff_note              TEXT,
            responder_label           TEXT,
            predecessor_read_only_grant TEXT NOT NULL,
            rendezvous_id             TEXT NOT NULL,
            recorded_at_ms            INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_emergency_handoffs_source
            ON _emergency_handoffs (source_case_ulid);
        CREATE INDEX IF NOT EXISTS idx_emergency_handoffs_recorded
            ON _emergency_handoffs (recorded_at_ms DESC);
        "#,
    )?;

    // Migration 001: per-OIDC gating columns. Idempotent — we ALTER TABLE
    // ADD COLUMN only when the column is missing. SQLite doesn't have
    // `ADD COLUMN IF NOT EXISTS`, so we introspect `pragma_table_info`.
    apply_migration_001_oidc_columns(conn)?;
    Ok(())
}

/// Migration 001: add `oidc_iss` and `oidc_sub` to `registrations`.
///
/// Both columns are nullable so existing rows upgrade cleanly. When
/// per-OIDC gating is on, new rows fill these in from the verified
/// `id_token`'s `iss`/`sub` claims. When permissive, they stay NULL.
fn apply_migration_001_oidc_columns(conn: &mut Connection) -> anyhow::Result<()> {
    let has_col = |conn: &Connection, col: &str| -> rusqlite::Result<bool> {
        let mut stmt =
            conn.prepare("SELECT 1 FROM pragma_table_info('registrations') WHERE name = ?1")?;
        let mut rows = stmt.query(params![col])?;
        Ok(rows.next()?.is_some())
    };
    if !has_col(conn, "oidc_iss")? {
        conn.execute_batch("ALTER TABLE registrations ADD COLUMN oidc_iss TEXT NULL;")?;
    }
    if !has_col(conn, "oidc_sub")? {
        conn.execute_batch("ALTER TABLE registrations ADD COLUMN oidc_sub TEXT NULL;")?;
    }
    Ok(())
}

fn log_event(
    conn: &Connection,
    rendezvous_id: &str,
    kind: &str,
    detail: Option<&str>,
) -> rusqlite::Result<()> {
    let now_ms = now_ms();
    conn.execute(
        "INSERT INTO registration_events (rendezvous_id, kind, detail, at_ms)
         VALUES (?1, ?2, ?3, ?4)",
        params![rendezvous_id, kind, detail, now_ms],
    )?;
    Ok(())
}

fn row_from_sql(r: &rusqlite::Row<'_>) -> anyhow::Result<RegistrationRow> {
    let rendezvous_id: String = r.get(0)?;
    let user_ulid_blob: Vec<u8> = r.get(1)?;
    let user_ulid: [u8; 16] = user_ulid_blob.try_into().map_err(|v: Vec<u8>| {
        anyhow::anyhow!("user_ulid blob length {} != 16", v.len())
    })?;
    let push_token_str: Option<String> = r.get(2)?;
    let push_token = push_token_str.as_deref().and_then(PushToken::decode);
    let last_heartbeat_at_ms: i64 = r.get(3)?;
    let cred_hash_blob: Vec<u8> = r.get(4)?;
    let long_lived_credential_hash: [u8; 32] =
        cred_hash_blob.try_into().map_err(|v: Vec<u8>| {
            anyhow::anyhow!("credential hash blob length {} != 32", v.len())
        })?;
    let registered_at_ms: i64 = r.get(5)?;
    let user_label: Option<String> = r.get(6)?;
    let storage_pubkey: Vec<u8> = r.get(7)?;
    let oidc_iss: Option<String> = r.get(8)?;
    let oidc_sub: Option<String> = r.get(9)?;

    Ok(RegistrationRow {
        rendezvous_id,
        user_ulid,
        push_token,
        last_heartbeat_at_ms,
        long_lived_credential_hash,
        registered_at_ms,
        user_label,
        storage_pubkey,
        oidc_iss,
        oidc_sub,
    })
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_row(id: &str, user_byte: u8) -> RegistrationRow {
        RegistrationRow {
            rendezvous_id: id.to_string(),
            user_ulid: [user_byte; 16],
            push_token: Some(PushToken::Fcm("token-abc".into())),
            last_heartbeat_at_ms: 1_700_000_000_000,
            long_lived_credential_hash: [0xAB; 32],
            registered_at_ms: 1_700_000_000_000,
            user_label: Some("test".into()),
            storage_pubkey: vec![0xCD; 32],
            oidc_iss: None,
            oidc_sub: None,
        }
    }

    #[tokio::test]
    async fn open_and_register_roundtrip() {
        let table = RegistrationTable::in_memory().await.unwrap();
        let row = sample_row("rzv-aaaa", 1);
        table.register(row.clone()).await.unwrap();

        let fetched = table.lookup_by_rendezvous("rzv-aaaa").await.unwrap().unwrap();
        assert_eq!(fetched, row);
        assert_eq!(table.count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn heartbeat_updates_timestamp() {
        let table = RegistrationTable::in_memory().await.unwrap();
        table.register(sample_row("rzv-bb", 2)).await.unwrap();

        let updated = table.heartbeat("rzv-bb", 9_999).await.unwrap();
        assert!(updated);

        let fetched = table.lookup_by_rendezvous("rzv-bb").await.unwrap().unwrap();
        assert_eq!(fetched.last_heartbeat_at_ms, 9_999);

        let missing = table.heartbeat("nope", 1).await.unwrap();
        assert!(!missing);
    }

    #[tokio::test]
    async fn deregister_removes_row_and_logs_event() {
        let table = RegistrationTable::in_memory().await.unwrap();
        table.register(sample_row("rzv-cc", 3)).await.unwrap();

        assert!(table.deregister("rzv-cc").await.unwrap());
        assert_eq!(table.count().await.unwrap(), 0);

        let events = table.recent_events(10).await.unwrap();
        // Should include at least register + deregister.
        let kinds: Vec<_> = events.iter().map(|e| e.kind.as_str()).collect();
        assert!(kinds.contains(&"register"));
        assert!(kinds.contains(&"deregister"));
    }

    #[tokio::test]
    async fn update_push_token_changes_value() {
        let table = RegistrationTable::in_memory().await.unwrap();
        table.register(sample_row("rzv-dd", 4)).await.unwrap();

        let new_token = Some(PushToken::Apns("apns-x".into()));
        assert!(table.update_push_token("rzv-dd", new_token.clone()).await.unwrap());

        let fetched = table.lookup_by_rendezvous("rzv-dd").await.unwrap().unwrap();
        assert_eq!(fetched.push_token, new_token);
    }

    #[tokio::test]
    async fn oidc_columns_persist_when_set() {
        let table = RegistrationTable::in_memory().await.unwrap();
        let mut row = sample_row("rzv-oidc", 7);
        row.oidc_iss = Some("https://idp.example".into());
        row.oidc_sub = Some("user@idp".into());
        table.register(row.clone()).await.unwrap();

        let fetched = table.lookup_by_rendezvous("rzv-oidc").await.unwrap().unwrap();
        assert_eq!(fetched.oidc_iss.as_deref(), Some("https://idp.example"));
        assert_eq!(fetched.oidc_sub.as_deref(), Some("user@idp"));
    }

    #[test]
    fn push_token_encode_decode_roundtrip() {
        let cases = vec![
            PushToken::Fcm("abc".into()),
            PushToken::Apns("xyz".into()),
            PushToken::Email("a@b.c".into()),
            PushToken::WebPush {
                endpoint: "https://push".into(),
                p256dh: "key".into(),
                auth: "auth".into(),
            },
        ];
        for tok in cases {
            let s = tok.encode();
            let back = PushToken::decode(&s).unwrap();
            assert_eq!(tok, back);
        }
    }
}
