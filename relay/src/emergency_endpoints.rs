//! Emergency-flow endpoints: status polling + handoff.
//!
//! These two endpoints sit alongside `POST /v1/emergency/initiate`
//! (in `server.rs`) and complete the operator-tablet break-glass loop:
//!
//! 1. Tablet calls `/v1/emergency/initiate` → relay signs the request,
//!    pushes to the patient phone, **records a `_emergency_requests` row
//!    with state=waiting**.
//! 2. Tablet polls `GET /v1/emergency/status/{request_id}` → relay reads
//!    the row, returns the current state.
//! 3. Patient phone (via OHD Connect) responds out-of-band; the relay's
//!    inbound notification handler (or the storage forwarder) flips the
//!    row to `approved` / `rejected` with `grant_token` + `case_ulid`.
//!    A background TTL sweeper transitions stale `waiting` rows to
//!    `auto_granted_timeout` (when the patient's emergency profile says
//!    so) or `expired`.
//! 4. After the responder closes the case, the tablet POSTs
//!    `/v1/emergency/handoff` to hand off to a successor operator
//!    (e.g. EMS hands the patient to Motol ER). The relay forwards to
//!    the patient's storage, which mints the successor case + a
//!    read-only grant for the predecessor.
//!
//! ## Why this lives outside `server.rs`
//!
//! `server.rs` is already a thousand-line file holding the registration
//! + tunnel + WS surface; the emergency state machine, table schema, and
//! TTL sweeper are self-contained enough to deserve their own module.
//! The two HTTP handlers are wired into `server.rs::build_router` (just
//! like `/v1/emergency/initiate` is) so the axum surface stays uniform.
//!
//! ## Storage tunnel for handoff (contract-level only in v1)
//!
//! `/v1/emergency/handoff` needs to invoke `OhdcService.HandoffCase` on
//! the patient's storage. The storage tunnel client crate isn't shipped
//! yet (it's the storage-side outbound integration tracked in
//! `STATUS.md`). We expose a [`StorageTunnelClient`] trait so the
//! production wiring can plug in once the tunnel client lands; tests
//! plug in [`MockStorageTunnel`] directly. Without a configured client
//! the relay returns 503 with `code=storage_tunnel_unavailable`.

#![allow(clippy::too_many_arguments)]

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use rand::RngCore;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

use crate::server::{ApiError, AppState};
use crate::state::now_ms;

// ---------------------------------------------------------------------------
// Defaults / constants
// ---------------------------------------------------------------------------

/// How long a freshly-issued emergency request stays in `waiting` before
/// the TTL sweeper transitions it to `auto_granted_timeout` (when the
/// patient's emergency profile says default-allow) or `expired` (when
/// not). Per the prompt: 30s.
pub const DEFAULT_REQUEST_TTL: Duration = Duration::from_secs(30);

/// Grace window after `expires_at_ms` before the row is GC'd. Per the
/// prompt: TTL grace + 5min.
pub const REQUEST_GC_GRACE: Duration = Duration::from_secs(5 * 60);

/// How often the TTL sweeper task wakes up. Short enough to feel
/// responsive in tests, long enough to be cheap in production.
pub const TTL_SWEEPER_TICK: Duration = Duration::from_secs(1);

// ---------------------------------------------------------------------------
// Wire shapes
// ---------------------------------------------------------------------------

/// Persisted state of an emergency request as the patient-side
/// negotiation progresses. The string form is what's serialized on the
/// wire and stored in SQLite (so the column is human-grep-able from
/// `sqlite3` for ops).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmergencyState {
    /// Initial state: relay pushed the signed request, awaiting the
    /// patient phone's accept/reject.
    Waiting,
    /// Patient accepted. `grant_token` + `case_ulid` populated.
    Approved,
    /// Patient explicitly rejected. `rejected_reason` may be populated.
    Rejected,
    /// TTL elapsed without a response AND the patient's emergency
    /// profile defaults to Allow. Same outcome as `approved` from the
    /// responder's POV (`grant_token` + `case_ulid` populated) but the
    /// audit trail is distinct.
    AutoGrantedTimeout,
    /// TTL elapsed without a response AND the patient's emergency
    /// profile defaults to Deny / no profile available. Terminal.
    Expired,
}

impl EmergencyState {
    pub fn as_db_str(self) -> &'static str {
        match self {
            EmergencyState::Waiting => "waiting",
            EmergencyState::Approved => "approved",
            EmergencyState::Rejected => "rejected",
            EmergencyState::AutoGrantedTimeout => "auto_granted_timeout",
            EmergencyState::Expired => "expired",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "waiting" => Some(EmergencyState::Waiting),
            "approved" => Some(EmergencyState::Approved),
            "rejected" => Some(EmergencyState::Rejected),
            "auto_granted_timeout" => Some(EmergencyState::AutoGrantedTimeout),
            "expired" => Some(EmergencyState::Expired),
            _ => None,
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, EmergencyState::Waiting)
    }

    /// True when the responder has effective access (real grant_token).
    pub fn has_grant(self) -> bool {
        matches!(
            self,
            EmergencyState::Approved | EmergencyState::AutoGrantedTimeout
        )
    }
}

/// Patient's emergency profile default action, surfaced from
/// `OhdcService.GetEmergencyConfig` and stored on the request row so the
/// TTL sweeper doesn't need to call back to storage when the timer
/// fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmergencyDefaultAction {
    /// Auto-grant on timeout — the common case for users who'd rather
    /// have first responders see their meds than have the access wait
    /// behind a dialog they can't tap.
    Allow,
    /// Stay locked when the timer fires.
    Deny,
}

impl EmergencyDefaultAction {
    pub fn as_db_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }

    pub fn from_db_str(s: &str) -> Option<Self> {
        match s {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Row types
// ---------------------------------------------------------------------------

/// Row in `_emergency_requests`. One per active break-glass request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmergencyRequestRow {
    pub request_id: String,
    pub rendezvous_id: String,
    pub state: EmergencyState,
    pub patient_label: Option<String>,
    pub grant_token: Option<String>,
    pub case_ulid: Option<String>,
    pub rejected_reason: Option<String>,
    pub default_action: Option<EmergencyDefaultAction>,
    pub created_at_ms: i64,
    pub decided_at_ms: Option<i64>,
    pub expires_at_ms: i64,
    pub gc_after_ms: i64,
}

/// Row in `_emergency_handoffs`. Operator-side audit; the storage holds
/// the canonical case state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmergencyHandoffRow {
    pub audit_entry_ulid: String,
    pub source_case_ulid: String,
    pub successor_case_ulid: String,
    pub target_operator: String,
    pub successor_operator_label: String,
    pub handoff_note: Option<String>,
    pub responder_label: Option<String>,
    pub predecessor_read_only_grant: String,
    pub rendezvous_id: String,
    pub recorded_at_ms: i64,
}

// ---------------------------------------------------------------------------
// Table — wraps the existing RegistrationTable's connection, but exposes
// the emergency CRUD surface separately so the dependency direction is
// clear (state.rs owns the schema; this module owns the access pattern).
// ---------------------------------------------------------------------------

/// Thin wrapper around the relay's SQLite connection that exposes the
/// `_emergency_requests` + `_emergency_handoffs` access surface. We
/// reuse the same `Connection` that `RegistrationTable` uses — it's
/// already wrapped in a `tokio::sync::Mutex`, and the schema migration
/// in `state.rs::init_schema` creates the tables this module touches.
#[derive(Clone)]
pub struct EmergencyStateTable {
    conn: Arc<Mutex<Connection>>,
}

impl EmergencyStateTable {
    pub fn new(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    // -- Requests -----------------------------------------------------------

    pub async fn insert_request(&self, row: EmergencyRequestRow) -> anyhow::Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO _emergency_requests (
                    request_id, rendezvous_id, state, patient_label, grant_token,
                    case_ulid, rejected_reason, default_action,
                    created_at_ms, decided_at_ms, expires_at_ms, gc_after_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![
                    row.request_id,
                    row.rendezvous_id,
                    row.state.as_db_str(),
                    row.patient_label,
                    row.grant_token,
                    row.case_ulid,
                    row.rejected_reason,
                    row.default_action.map(|d| d.as_db_str()),
                    row.created_at_ms,
                    row.decided_at_ms,
                    row.expires_at_ms,
                    row.gc_after_ms,
                ],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn lookup_request(
        &self,
        request_id: &str,
    ) -> anyhow::Result<Option<EmergencyRequestRow>> {
        let conn = self.conn.clone();
        let id = request_id.to_string();
        tokio::task::spawn_blocking(move || -> anyhow::Result<Option<EmergencyRequestRow>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT request_id, rendezvous_id, state, patient_label, grant_token,
                        case_ulid, rejected_reason, default_action,
                        created_at_ms, decided_at_ms, expires_at_ms, gc_after_ms
                 FROM _emergency_requests WHERE request_id = ?1",
            )?;
            let mut rows = stmt.query(params![id])?;
            if let Some(r) = rows.next()? {
                Ok(Some(request_row_from_sql(r)?))
            } else {
                Ok(None)
            }
        })
        .await?
    }

    /// Approve a waiting request. No-op (returns false) when the row is
    /// already terminal.
    pub async fn approve_request(
        &self,
        request_id: &str,
        grant_token: String,
        case_ulid: String,
        patient_label: Option<String>,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        self.transition(
            request_id.to_string(),
            EmergencyState::Approved,
            Some(now_ms),
            Some(grant_token),
            Some(case_ulid),
            patient_label,
            None,
        )
        .await
    }

    /// Reject a waiting request. No-op when terminal.
    pub async fn reject_request(
        &self,
        request_id: &str,
        reason: Option<String>,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        self.transition(
            request_id.to_string(),
            EmergencyState::Rejected,
            Some(now_ms),
            None,
            None,
            None,
            reason,
        )
        .await
    }

    /// Auto-grant on timeout. Used by the TTL sweeper.
    pub async fn auto_grant_request(
        &self,
        request_id: &str,
        grant_token: String,
        case_ulid: String,
        patient_label: Option<String>,
        now_ms: i64,
    ) -> anyhow::Result<bool> {
        self.transition(
            request_id.to_string(),
            EmergencyState::AutoGrantedTimeout,
            Some(now_ms),
            Some(grant_token),
            Some(case_ulid),
            patient_label,
            None,
        )
        .await
    }

    /// Expire a waiting request — TTL elapsed and patient's profile says
    /// no.
    pub async fn expire_request(&self, request_id: &str, now_ms: i64) -> anyhow::Result<bool> {
        self.transition(
            request_id.to_string(),
            EmergencyState::Expired,
            Some(now_ms),
            None,
            None,
            None,
            Some("ttl_elapsed".to_string()),
        )
        .await
    }

    /// Generic transition. Only mutates rows currently in `waiting`;
    /// returning `false` indicates either "no such row" or "already
    /// terminal".
    async fn transition(
        &self,
        request_id: String,
        next: EmergencyState,
        decided_at_ms: Option<i64>,
        grant_token: Option<String>,
        case_ulid: Option<String>,
        patient_label: Option<String>,
        rejected_reason: Option<String>,
    ) -> anyhow::Result<bool> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<bool> {
            let conn = conn.blocking_lock();
            // Surgical UPDATE: COALESCE preserves existing values when
            // the caller doesn't pass a new one (e.g. `expire_request`
            // doesn't touch `patient_label`).
            let updated = conn.execute(
                "UPDATE _emergency_requests
                 SET state = ?1,
                     decided_at_ms = ?2,
                     grant_token = COALESCE(?3, grant_token),
                     case_ulid = COALESCE(?4, case_ulid),
                     patient_label = COALESCE(?5, patient_label),
                     rejected_reason = COALESCE(?6, rejected_reason)
                 WHERE request_id = ?7 AND state = 'waiting'",
                params![
                    next.as_db_str(),
                    decided_at_ms,
                    grant_token,
                    case_ulid,
                    patient_label,
                    rejected_reason,
                    request_id,
                ],
            )?;
            Ok(updated > 0)
        })
        .await?
    }

    /// List all requests whose `expires_at_ms <= now` and are still in
    /// `waiting` state. The TTL sweeper consumes this.
    pub async fn list_expired_waiting(
        &self,
        now_ms: i64,
    ) -> anyhow::Result<Vec<EmergencyRequestRow>> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<EmergencyRequestRow>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT request_id, rendezvous_id, state, patient_label, grant_token,
                        case_ulid, rejected_reason, default_action,
                        created_at_ms, decided_at_ms, expires_at_ms, gc_after_ms
                 FROM _emergency_requests
                 WHERE state = 'waiting' AND expires_at_ms <= ?1",
            )?;
            let rows = stmt.query_map(params![now_ms], |r| {
                request_row_from_sql(r).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())),
                    )
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

    /// GC: drop rows whose `gc_after_ms <= now`. Called by the sweeper
    /// after rows have been terminal long enough.
    pub async fn gc_old_requests(&self, now_ms: i64) -> anyhow::Result<u64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<u64> {
            let conn = conn.blocking_lock();
            let n = conn.execute(
                "DELETE FROM _emergency_requests WHERE gc_after_ms <= ?1",
                params![now_ms],
            )?;
            Ok(n as u64)
        })
        .await?
    }

    pub async fn count_requests(&self) -> anyhow::Result<i64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
            let conn = conn.blocking_lock();
            let n: i64 =
                conn.query_row("SELECT COUNT(*) FROM _emergency_requests", [], |r| r.get(0))?;
            Ok(n)
        })
        .await?
    }

    // -- Handoffs -----------------------------------------------------------

    pub async fn insert_handoff(&self, row: EmergencyHandoffRow) -> anyhow::Result<()> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            let conn = conn.blocking_lock();
            conn.execute(
                "INSERT INTO _emergency_handoffs (
                    audit_entry_ulid, source_case_ulid, successor_case_ulid,
                    target_operator, successor_operator_label,
                    handoff_note, responder_label,
                    predecessor_read_only_grant, rendezvous_id, recorded_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    row.audit_entry_ulid,
                    row.source_case_ulid,
                    row.successor_case_ulid,
                    row.target_operator,
                    row.successor_operator_label,
                    row.handoff_note,
                    row.responder_label,
                    row.predecessor_read_only_grant,
                    row.rendezvous_id,
                    row.recorded_at_ms,
                ],
            )?;
            Ok(())
        })
        .await?
    }

    pub async fn count_handoffs(&self) -> anyhow::Result<i64> {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<i64> {
            let conn = conn.blocking_lock();
            let n: i64 =
                conn.query_row("SELECT COUNT(*) FROM _emergency_handoffs", [], |r| r.get(0))?;
            Ok(n)
        })
        .await?
    }

    pub async fn lookup_handoff_by_source(
        &self,
        source_case_ulid: &str,
    ) -> anyhow::Result<Option<EmergencyHandoffRow>> {
        let conn = self.conn.clone();
        let id = source_case_ulid.to_string();
        tokio::task::spawn_blocking(move || -> anyhow::Result<Option<EmergencyHandoffRow>> {
            let conn = conn.blocking_lock();
            let mut stmt = conn.prepare(
                "SELECT audit_entry_ulid, source_case_ulid, successor_case_ulid,
                        target_operator, successor_operator_label,
                        handoff_note, responder_label,
                        predecessor_read_only_grant, rendezvous_id, recorded_at_ms
                 FROM _emergency_handoffs WHERE source_case_ulid = ?1
                 ORDER BY recorded_at_ms DESC LIMIT 1",
            )?;
            let mut rows = stmt.query(params![id])?;
            if let Some(r) = rows.next()? {
                Ok(Some(handoff_row_from_sql(r)?))
            } else {
                Ok(None)
            }
        })
        .await?
    }
}

fn request_row_from_sql(r: &rusqlite::Row<'_>) -> anyhow::Result<EmergencyRequestRow> {
    let state_str: String = r.get(2)?;
    let state = EmergencyState::from_db_str(&state_str)
        .ok_or_else(|| anyhow::anyhow!("unknown emergency state in db: {state_str}"))?;
    let default_action_str: Option<String> = r.get(7)?;
    let default_action = default_action_str
        .as_deref()
        .and_then(EmergencyDefaultAction::from_db_str);
    Ok(EmergencyRequestRow {
        request_id: r.get(0)?,
        rendezvous_id: r.get(1)?,
        state,
        patient_label: r.get(3)?,
        grant_token: r.get(4)?,
        case_ulid: r.get(5)?,
        rejected_reason: r.get(6)?,
        default_action,
        created_at_ms: r.get(8)?,
        decided_at_ms: r.get(9)?,
        expires_at_ms: r.get(10)?,
        gc_after_ms: r.get(11)?,
    })
}

fn handoff_row_from_sql(r: &rusqlite::Row<'_>) -> anyhow::Result<EmergencyHandoffRow> {
    Ok(EmergencyHandoffRow {
        audit_entry_ulid: r.get(0)?,
        source_case_ulid: r.get(1)?,
        successor_case_ulid: r.get(2)?,
        target_operator: r.get(3)?,
        successor_operator_label: r.get(4)?,
        handoff_note: r.get(5)?,
        responder_label: r.get(6)?,
        predecessor_read_only_grant: r.get(7)?,
        rendezvous_id: r.get(8)?,
        recorded_at_ms: r.get(9)?,
    })
}

// ---------------------------------------------------------------------------
// Storage tunnel client trait (handoff + GetEmergencyConfig)
// ---------------------------------------------------------------------------

/// Outbound RPCs the relay needs to invoke on the patient's storage to
/// service these endpoints. Decoupled behind a trait so we can:
///
/// - plug in a real Connect-RPC-over-tunnel client when it lands (tracked
///   in `STATUS.md` "What's stubbed / TBD" — the storage-side outbound
///   integration); and
/// - drop in [`MockStorageTunnel`] in unit + integration tests without
///   booting the WS / QUIC tunnel.
///
/// Implementations should be cheaply cloneable; the trait extends
/// `Send + Sync` so it threads through axum's `AppState`.
#[async_trait]
pub trait StorageTunnelClient: Send + Sync {
    /// Invoke `OhdcService.HandoffCase(source_case, target_operator,
    /// note, responder_label)` on the patient's storage at
    /// `rendezvous_id`. Returns the patient-side response or a wire
    /// error.
    async fn handoff_case(
        &self,
        rendezvous_id: &str,
        req: HandoffCaseRequest,
    ) -> Result<HandoffCaseResponse, StorageTunnelError>;

    /// Invoke `OhdcService.GetEmergencyConfig()` on the patient's
    /// storage. Returns the patient's emergency profile (default action +
    /// label).
    async fn get_emergency_config(
        &self,
        rendezvous_id: &str,
    ) -> Result<EmergencyConfigResponse, StorageTunnelError>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HandoffCaseRequest {
    pub source_case_ulid: String,
    pub target_operator: String,
    pub handoff_note: Option<String>,
    pub responder_label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HandoffCaseResponse {
    pub successor_case_ulid: String,
    pub successor_operator_label: String,
    pub predecessor_read_only_grant: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmergencyConfigResponse {
    pub default_action: EmergencyDefaultAction,
    pub patient_label: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum StorageTunnelError {
    #[error("storage tunnel unavailable for rendezvous_id={0}")]
    Unavailable(String),
    #[error("storage rejected: {0}")]
    Rejected(String),
    #[error("storage tunnel io: {0}")]
    Io(String),
    #[error("storage tunnel client not configured on this relay")]
    NotConfigured,
}

/// In-memory mock used by tests + the placeholder production wiring
/// until the real tunnel client lands. Drives the contract by
/// pre-registering canned responses keyed by `rendezvous_id`.
#[derive(Default, Clone)]
pub struct MockStorageTunnel {
    inner: Arc<Mutex<MockState>>,
}

#[derive(Default)]
struct MockState {
    handoff: std::collections::HashMap<String, HandoffCaseResponse>,
    config: std::collections::HashMap<String, EmergencyConfigResponse>,
    /// Optional canned errors: when set for a rendezvous_id, the call
    /// returns the error instead of looking up the canned response.
    handoff_errors: std::collections::HashMap<String, String>,
    config_errors: std::collections::HashMap<String, String>,
}

impl MockStorageTunnel {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn set_handoff(&self, rendezvous_id: &str, resp: HandoffCaseResponse) {
        self.inner
            .lock()
            .await
            .handoff
            .insert(rendezvous_id.to_string(), resp);
    }

    pub async fn set_config(&self, rendezvous_id: &str, resp: EmergencyConfigResponse) {
        self.inner
            .lock()
            .await
            .config
            .insert(rendezvous_id.to_string(), resp);
    }

    pub async fn fail_handoff(&self, rendezvous_id: &str, msg: impl Into<String>) {
        self.inner
            .lock()
            .await
            .handoff_errors
            .insert(rendezvous_id.to_string(), msg.into());
    }

    pub async fn fail_config(&self, rendezvous_id: &str, msg: impl Into<String>) {
        self.inner
            .lock()
            .await
            .config_errors
            .insert(rendezvous_id.to_string(), msg.into());
    }
}

#[async_trait]
impl StorageTunnelClient for MockStorageTunnel {
    async fn handoff_case(
        &self,
        rendezvous_id: &str,
        _req: HandoffCaseRequest,
    ) -> Result<HandoffCaseResponse, StorageTunnelError> {
        let g = self.inner.lock().await;
        if let Some(err) = g.handoff_errors.get(rendezvous_id) {
            return Err(StorageTunnelError::Rejected(err.clone()));
        }
        g.handoff
            .get(rendezvous_id)
            .cloned()
            .ok_or_else(|| StorageTunnelError::Unavailable(rendezvous_id.to_string()))
    }

    async fn get_emergency_config(
        &self,
        rendezvous_id: &str,
    ) -> Result<EmergencyConfigResponse, StorageTunnelError> {
        let g = self.inner.lock().await;
        if let Some(err) = g.config_errors.get(rendezvous_id) {
            return Err(StorageTunnelError::Rejected(err.clone()));
        }
        g.config
            .get(rendezvous_id)
            .cloned()
            .ok_or_else(|| StorageTunnelError::Unavailable(rendezvous_id.to_string()))
    }
}

// ---------------------------------------------------------------------------
// HTTP handlers — `/v1/emergency/status/{request_id}` + `/v1/emergency/handoff`
// ---------------------------------------------------------------------------

/// Wire response for `GET /v1/emergency/status/{request_id}`.
///
/// Mirrors the prompt's documented shape:
/// ```json
/// {
///   "request_id": "...",
///   "state": "waiting" | "approved" | "rejected" | "auto_granted_timeout" | "expired",
///   "patient_label": "...",
///   "grant_token": "ohdg_...",
///   "case_ulid": "01HX...",
///   "created_at_ms": ...,
///   "decided_at_ms": ...,
///   "expires_at_ms": ...
/// }
/// ```
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EmergencyStatusResponse {
    pub request_id: String,
    pub state: EmergencyState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patient_label: Option<String>,
    /// Only present when state is `approved` / `auto_granted_timeout`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_token: Option<String>,
    /// Same gating as `grant_token`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_ulid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected_reason: Option<String>,
    pub created_at_ms: i64,
    /// Null when state is `waiting`.
    pub decided_at_ms: Option<i64>,
    pub expires_at_ms: i64,
}

impl From<EmergencyRequestRow> for EmergencyStatusResponse {
    fn from(row: EmergencyRequestRow) -> Self {
        // Hide grant_token + case_ulid unless the responder has effective
        // access (approved or auto_granted_timeout). Defensive even
        // though `transition` only writes them in those states.
        let (grant_token, case_ulid) = if row.state.has_grant() {
            (row.grant_token, row.case_ulid)
        } else {
            (None, None)
        };
        Self {
            request_id: row.request_id,
            state: row.state,
            patient_label: row.patient_label,
            grant_token,
            case_ulid,
            rejected_reason: row.rejected_reason,
            created_at_ms: row.created_at_ms,
            decided_at_ms: row.decided_at_ms,
            expires_at_ms: row.expires_at_ms,
        }
    }
}

pub async fn handle_emergency_status(
    State(state): State<AppState>,
    Path(request_id): Path<String>,
) -> Result<Json<EmergencyStatusResponse>, ApiError> {
    let row = state
        .emergency
        .lookup_request(&request_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| ApiError::not_found("request_id"))?;
    Ok(Json(row.into()))
}

/// Wire request for `POST /v1/emergency/handoff`.
#[derive(Debug, Deserialize, Serialize)]
pub struct EmergencyHandoffRequest {
    pub source_case_ulid: String,
    pub target_operator: String,
    pub handoff_note: Option<String>,
    pub responder_label: Option<String>,
    /// Optional rendezvous hint. The relay can also resolve the
    /// rendezvous via the case-rendezvous binding it tracks (when the
    /// initiate step recorded it); when absent and unresolvable, the
    /// handler returns 404 with `code=case_not_active`.
    pub rendezvous_id: Option<String>,
}

/// Wire response for `POST /v1/emergency/handoff`. Field names match
/// what `OhdcClient.kt::HandoffResponseDto` reads, plus the explicit
/// `successor_operator_label` + `audit_entry_ulid` per the prompt.
///
/// We emit BOTH `predecessor_read_only_grant` (prompt) and
/// `read_only_grant_token` (current tablet DTO) so the tablet doesn't
/// need an in-flight code change to consume the wire.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct EmergencyHandoffResponse {
    pub successor_case_ulid: String,
    pub successor_operator_label: String,
    pub predecessor_read_only_grant: String,
    /// Compatibility alias for the existing tablet DTO; identical to
    /// `predecessor_read_only_grant`.
    pub read_only_grant_token: String,
    pub audit_entry_ulid: String,
}

pub async fn handle_emergency_handoff(
    State(state): State<AppState>,
    Json(req): Json<EmergencyHandoffRequest>,
) -> Result<Json<EmergencyHandoffResponse>, ApiError> {
    // Resolve which rendezvous to forward to. Either the request says,
    // or we look up the most recent emergency request that touched this
    // case (the tablet's flow records it on approval).
    let rendezvous_id = match req.rendezvous_id.clone() {
        Some(r) => r,
        None => state
            .emergency
            .lookup_handoff_by_source(&req.source_case_ulid)
            .await
            .map_err(ApiError::internal)?
            .map(|r| r.rendezvous_id)
            .ok_or_else(|| {
                ApiError::not_found("case not active under this relay")
                    .with_code("case_not_active")
            })?,
    };

    // Confirm the rendezvous is registered + recently active.
    let _registration = state
        .relay
        .registrations
        .lookup_by_rendezvous(&rendezvous_id)
        .await
        .map_err(ApiError::internal)?
        .ok_or_else(|| {
            ApiError::not_found("rendezvous_id not registered with this relay")
                .with_code("rendezvous_unknown")
        })?;

    // Forward to the patient's storage via the storage tunnel. When the
    // tunnel client is not yet wired (the v1 reality), surface 503 so
    // the tablet can fall back to its mock-handoff path cleanly.
    let tunnel = state
        .storage_tunnel
        .clone()
        .ok_or_else(|| {
            ApiError {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message:
                    "storage tunnel client not configured; handoff cannot be forwarded".into(),
                code: Some("storage_tunnel_unavailable"),
            }
        })?;

    let storage_resp = tunnel
        .handoff_case(
            &rendezvous_id,
            HandoffCaseRequest {
                source_case_ulid: req.source_case_ulid.clone(),
                target_operator: req.target_operator.clone(),
                handoff_note: req.handoff_note.clone(),
                responder_label: req.responder_label.clone(),
            },
        )
        .await
        .map_err(map_storage_tunnel_error)?;

    // Mint the audit entry.
    let audit_entry_ulid = generate_ulid_like();
    let row = EmergencyHandoffRow {
        audit_entry_ulid: audit_entry_ulid.clone(),
        source_case_ulid: req.source_case_ulid.clone(),
        successor_case_ulid: storage_resp.successor_case_ulid.clone(),
        target_operator: req.target_operator.clone(),
        successor_operator_label: storage_resp.successor_operator_label.clone(),
        handoff_note: req.handoff_note.clone(),
        responder_label: req.responder_label.clone(),
        predecessor_read_only_grant: storage_resp.predecessor_read_only_grant.clone(),
        rendezvous_id: rendezvous_id.clone(),
        recorded_at_ms: now_ms(),
    };

    state
        .emergency
        .insert_handoff(row)
        .await
        .map_err(ApiError::internal)?;

    info!(
        target: "ohd_relay::emergency",
        source = %req.source_case_ulid,
        successor = %storage_resp.successor_case_ulid,
        target_operator = %req.target_operator,
        %audit_entry_ulid,
        "emergency handoff recorded"
    );

    Ok(Json(EmergencyHandoffResponse {
        successor_case_ulid: storage_resp.successor_case_ulid.clone(),
        successor_operator_label: storage_resp.successor_operator_label,
        predecessor_read_only_grant: storage_resp.predecessor_read_only_grant.clone(),
        read_only_grant_token: storage_resp.predecessor_read_only_grant,
        audit_entry_ulid,
    }))
}

fn map_storage_tunnel_error(e: StorageTunnelError) -> ApiError {
    match e {
        StorageTunnelError::Unavailable(_) => ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: e.to_string(),
            code: Some("storage_tunnel_unavailable"),
        },
        StorageTunnelError::NotConfigured => ApiError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            message: e.to_string(),
            code: Some("storage_tunnel_unavailable"),
        },
        StorageTunnelError::Rejected(_) => ApiError {
            status: StatusCode::BAD_GATEWAY,
            message: e.to_string(),
            code: Some("storage_rejected"),
        },
        StorageTunnelError::Io(_) => ApiError {
            status: StatusCode::BAD_GATEWAY,
            message: e.to_string(),
            code: Some("storage_io_error"),
        },
    }
}

// ---------------------------------------------------------------------------
// Initiate-side helper: record + schedule TTL
// ---------------------------------------------------------------------------

/// Inserter used by `/v1/emergency/initiate` to register a new request
/// alongside the signed payload. Pulls the patient's emergency profile
/// (best-effort) so the TTL sweeper has a default-action to apply
/// without going back to storage when the timer fires.
pub async fn record_initiated_request(
    table: &EmergencyStateTable,
    storage_tunnel: Option<&Arc<dyn StorageTunnelClient>>,
    request_id: String,
    rendezvous_id: String,
    expires_at_ms: i64,
    now_ms: i64,
) -> anyhow::Result<EmergencyRequestRow> {
    // Best-effort GetEmergencyConfig: when the storage tunnel is up,
    // pull the patient's default-action so we know what to do if the
    // timer fires. When unavailable, stash `None` and treat absent as
    // Deny (fail-closed) when the sweeper actually fires.
    let (default_action, patient_label) = match storage_tunnel {
        Some(t) => match t.get_emergency_config(&rendezvous_id).await {
            Ok(cfg) => (Some(cfg.default_action), cfg.patient_label),
            Err(e) => {
                debug!(
                    target: "ohd_relay::emergency",
                    %rendezvous_id,
                    error = %e,
                    "GetEmergencyConfig unavailable; TTL will fail-closed"
                );
                (None, None)
            }
        },
        None => (None, None),
    };

    let row = EmergencyRequestRow {
        request_id: request_id.clone(),
        rendezvous_id,
        state: EmergencyState::Waiting,
        patient_label,
        grant_token: None,
        case_ulid: None,
        rejected_reason: None,
        default_action,
        created_at_ms: now_ms,
        decided_at_ms: None,
        expires_at_ms,
        gc_after_ms: expires_at_ms + REQUEST_GC_GRACE.as_millis() as i64,
    };
    table.insert_request(row.clone()).await?;
    Ok(row)
}

// ---------------------------------------------------------------------------
// TTL sweeper
// ---------------------------------------------------------------------------

/// Single sweeper tick: process all expired-waiting rows + GC stale
/// terminal rows. Exposed so tests can drive deterministically without
/// `tokio::time::pause`.
pub async fn run_ttl_sweep(
    table: &EmergencyStateTable,
    now_ms: i64,
) -> anyhow::Result<TtlSweepStats> {
    let expired = table.list_expired_waiting(now_ms).await?;
    let mut auto_granted = 0u64;
    let mut expired_count = 0u64;
    for row in expired {
        match row.default_action {
            Some(EmergencyDefaultAction::Allow) => {
                let grant = format!("ohdg_{}", random_token_suffix());
                let case = generate_ulid_like();
                let did = table
                    .auto_grant_request(
                        &row.request_id,
                        grant,
                        case,
                        row.patient_label.clone(),
                        now_ms,
                    )
                    .await?;
                if did {
                    auto_granted += 1;
                }
            }
            Some(EmergencyDefaultAction::Deny) | None => {
                // No profile available → fail-closed (expire). The
                // patient's emergency profile is the source of truth;
                // when the relay can't read it we don't auto-grant.
                let did = table.expire_request(&row.request_id, now_ms).await?;
                if did {
                    expired_count += 1;
                }
            }
        }
    }
    let gc_count = table.gc_old_requests(now_ms).await?;
    Ok(TtlSweepStats {
        auto_granted,
        expired: expired_count,
        gc_count,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TtlSweepStats {
    pub auto_granted: u64,
    pub expired: u64,
    pub gc_count: u64,
}

/// Background loop. Runs `run_ttl_sweep` on `TTL_SWEEPER_TICK` cadence.
/// Bail out gracefully on `shutdown` (a `tokio::sync::watch::Receiver`).
pub async fn run_ttl_sweeper_loop(
    table: EmergencyStateTable,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut ticker = tokio::time::interval(TTL_SWEEPER_TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                match run_ttl_sweep(&table, now_ms()).await {
                    Ok(stats) if stats.auto_granted + stats.expired + stats.gc_count > 0 => {
                        debug!(
                            target: "ohd_relay::emergency",
                            ?stats,
                            "ttl sweep tick"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!(
                            target: "ohd_relay::emergency",
                            error = %e,
                            "ttl sweep failed"
                        );
                    }
                }
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Crockford-style 26-char ULID-shaped string. Distinct from the
/// rendezvous-id helper (different alphabet not required, but the case
/// + audit ULIDs are a different namespace; keeping the helper local
/// makes that obvious in grep).
fn generate_ulid_like() -> String {
    const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    let now_ms_be = (now_ms() as u64).to_be_bytes();
    // First 6 bytes = timestamp, next 10 = random — same shape as a
    // canonical ULID.
    let mut combined = [0u8; 16];
    combined[..6].copy_from_slice(&now_ms_be[2..]);
    combined[6..].copy_from_slice(&buf[..10]);
    let mut out = String::with_capacity(26);
    let mut bits: u32 = 0;
    let mut bit_count = 0u32;
    for &b in &combined {
        bits = (bits << 8) | b as u32;
        bit_count += 8;
        while bit_count >= 5 {
            bit_count -= 5;
            let idx = (bits >> bit_count) & 0x1F;
            out.push(ALPHABET[idx as usize] as char);
        }
    }
    if bit_count > 0 {
        let idx = (bits << (5 - bit_count)) & 0x1F;
        out.push(ALPHABET[idx as usize] as char);
    }
    out.truncate(26);
    out
}

fn random_token_suffix() -> String {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut buf = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut buf);
    let mut out = String::with_capacity(32);
    let mut bits: u32 = 0;
    let mut bit_count = 0u32;
    for &b in &buf {
        bits = (bits << 8) | b as u32;
        bit_count += 8;
        while bit_count >= 5 {
            bit_count -= 5;
            let idx = (bits >> bit_count) & 0x1F;
            out.push(ALPHABET[idx as usize] as char);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::RelayState;

    async fn fresh_table() -> EmergencyStateTable {
        let relay = RelayState::in_memory().await.unwrap();
        EmergencyStateTable::new(relay.registrations.conn_for_emergency())
    }

    fn fresh_request(req_id: &str, ttl_ms: i64) -> EmergencyRequestRow {
        let now = 1_700_000_000_000_i64;
        EmergencyRequestRow {
            request_id: req_id.into(),
            rendezvous_id: "rzv-emerg".into(),
            state: EmergencyState::Waiting,
            patient_label: None,
            grant_token: None,
            case_ulid: None,
            rejected_reason: None,
            default_action: None,
            created_at_ms: now,
            decided_at_ms: None,
            expires_at_ms: now + ttl_ms,
            gc_after_ms: now + ttl_ms + REQUEST_GC_GRACE.as_millis() as i64,
        }
    }

    #[tokio::test]
    async fn migration_creates_emergency_tables() {
        let table = fresh_table().await;
        assert_eq!(table.count_requests().await.unwrap(), 0);
        assert_eq!(table.count_handoffs().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn insert_and_lookup_request_roundtrip() {
        let table = fresh_table().await;
        let row = fresh_request("req-1", 30_000);
        table.insert_request(row.clone()).await.unwrap();
        let fetched = table.lookup_request("req-1").await.unwrap().unwrap();
        assert_eq!(fetched, row);
        assert!(table.lookup_request("nope").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn approve_then_lookup_returns_grant() {
        let table = fresh_table().await;
        table.insert_request(fresh_request("req-2", 30_000)).await.unwrap();
        let did = table
            .approve_request(
                "req-2",
                "ohdg_TESTTOKEN".into(),
                "01HX_CASE".into(),
                Some("patient_label".into()),
                1_700_000_010_000,
            )
            .await
            .unwrap();
        assert!(did);
        let fetched = table.lookup_request("req-2").await.unwrap().unwrap();
        assert_eq!(fetched.state, EmergencyState::Approved);
        assert_eq!(fetched.grant_token.as_deref(), Some("ohdg_TESTTOKEN"));
        assert_eq!(fetched.case_ulid.as_deref(), Some("01HX_CASE"));
        assert_eq!(fetched.decided_at_ms, Some(1_700_000_010_000));
    }

    #[tokio::test]
    async fn cannot_double_transition() {
        let table = fresh_table().await;
        table.insert_request(fresh_request("req-3", 30_000)).await.unwrap();
        let first = table
            .approve_request(
                "req-3",
                "ohdg_X".into(),
                "01HX".into(),
                None,
                1_700_000_010_000,
            )
            .await
            .unwrap();
        assert!(first);
        // Attempting to reject after approval is a no-op.
        let second = table
            .reject_request("req-3", Some("late".into()), 1_700_000_020_000)
            .await
            .unwrap();
        assert!(!second);
        let row = table.lookup_request("req-3").await.unwrap().unwrap();
        assert_eq!(row.state, EmergencyState::Approved);
        assert_eq!(row.rejected_reason, None);
    }

    #[tokio::test]
    async fn ttl_sweep_auto_grants_when_default_allow() {
        let table = fresh_table().await;
        let mut row = fresh_request("req-4", 30_000);
        row.default_action = Some(EmergencyDefaultAction::Allow);
        table.insert_request(row.clone()).await.unwrap();

        // Sweep at row.expires_at_ms — the row should flip to
        // auto_granted_timeout.
        let stats = run_ttl_sweep(&table, row.expires_at_ms).await.unwrap();
        assert_eq!(stats.auto_granted, 1);
        assert_eq!(stats.expired, 0);

        let fetched = table.lookup_request("req-4").await.unwrap().unwrap();
        assert_eq!(fetched.state, EmergencyState::AutoGrantedTimeout);
        assert!(fetched.grant_token.is_some());
        assert!(fetched.case_ulid.is_some());
    }

    #[tokio::test]
    async fn ttl_sweep_expires_when_default_deny() {
        let table = fresh_table().await;
        let mut row = fresh_request("req-5", 30_000);
        row.default_action = Some(EmergencyDefaultAction::Deny);
        table.insert_request(row.clone()).await.unwrap();
        let stats = run_ttl_sweep(&table, row.expires_at_ms).await.unwrap();
        assert_eq!(stats.expired, 1);
        let fetched = table.lookup_request("req-5").await.unwrap().unwrap();
        assert_eq!(fetched.state, EmergencyState::Expired);
        assert_eq!(fetched.grant_token, None);
    }

    #[tokio::test]
    async fn ttl_sweep_expires_when_no_profile_known() {
        // No emergency profile loaded → fail-closed.
        let table = fresh_table().await;
        let row = fresh_request("req-6", 30_000);
        table.insert_request(row.clone()).await.unwrap();
        let stats = run_ttl_sweep(&table, row.expires_at_ms).await.unwrap();
        assert_eq!(stats.expired, 1);
        let fetched = table.lookup_request("req-6").await.unwrap().unwrap();
        assert_eq!(fetched.state, EmergencyState::Expired);
    }

    #[tokio::test]
    async fn ttl_sweep_skips_unexpired() {
        let table = fresh_table().await;
        let row = fresh_request("req-7", 30_000);
        table.insert_request(row.clone()).await.unwrap();
        let stats = run_ttl_sweep(&table, row.expires_at_ms - 1).await.unwrap();
        assert_eq!(stats.auto_granted + stats.expired, 0);
        let fetched = table.lookup_request("req-7").await.unwrap().unwrap();
        assert_eq!(fetched.state, EmergencyState::Waiting);
    }

    #[tokio::test]
    async fn gc_drops_old_terminal_rows() {
        let table = fresh_table().await;
        let row = fresh_request("req-8", 30_000);
        table.insert_request(row.clone()).await.unwrap();
        // Advance to past gc_after_ms; sweep deletes the row.
        let stats = run_ttl_sweep(&table, row.gc_after_ms + 1).await.unwrap();
        assert_eq!(stats.gc_count, 1);
        assert!(table.lookup_request("req-8").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn handoff_insert_and_lookup() {
        let table = fresh_table().await;
        let row = EmergencyHandoffRow {
            audit_entry_ulid: "audit-1".into(),
            source_case_ulid: "case-A".into(),
            successor_case_ulid: "case-B".into(),
            target_operator: "Motol ER".into(),
            successor_operator_label: "Motol ER".into(),
            handoff_note: Some("intubated".into()),
            responder_label: Some("P. Horak".into()),
            predecessor_read_only_grant: "ohdg_RO".into(),
            rendezvous_id: "rzv-emerg".into(),
            recorded_at_ms: 1_700_000_030_000,
        };
        table.insert_handoff(row.clone()).await.unwrap();
        let fetched = table
            .lookup_handoff_by_source("case-A")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched, row);
    }

    #[tokio::test]
    async fn mock_storage_tunnel_handoff_roundtrip() {
        let mock = MockStorageTunnel::new();
        mock.set_handoff(
            "rzv-emerg",
            HandoffCaseResponse {
                successor_case_ulid: "01HY".into(),
                successor_operator_label: "Motol ER".into(),
                predecessor_read_only_grant: "ohdg_RO".into(),
            },
        )
        .await;
        let resp = mock
            .handoff_case(
                "rzv-emerg",
                HandoffCaseRequest {
                    source_case_ulid: "01HX".into(),
                    target_operator: "Motol ER".into(),
                    handoff_note: None,
                    responder_label: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(resp.successor_case_ulid, "01HY");
    }

    #[tokio::test]
    async fn mock_storage_tunnel_handoff_unavailable() {
        let mock = MockStorageTunnel::new();
        let r = mock
            .handoff_case(
                "rzv-unknown",
                HandoffCaseRequest {
                    source_case_ulid: "01HX".into(),
                    target_operator: "Motol ER".into(),
                    handoff_note: None,
                    responder_label: None,
                },
            )
            .await;
        assert!(matches!(r, Err(StorageTunnelError::Unavailable(_))));
    }

    #[tokio::test]
    async fn emergency_state_strings_roundtrip() {
        for s in [
            EmergencyState::Waiting,
            EmergencyState::Approved,
            EmergencyState::Rejected,
            EmergencyState::AutoGrantedTimeout,
            EmergencyState::Expired,
        ] {
            assert_eq!(EmergencyState::from_db_str(s.as_db_str()), Some(s));
        }
        for d in [EmergencyDefaultAction::Allow, EmergencyDefaultAction::Deny] {
            assert_eq!(EmergencyDefaultAction::from_db_str(d.as_db_str()), Some(d));
        }
    }
}
