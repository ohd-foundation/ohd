//! Audit log.
//!
//! Backs `audit_log`. Every external operation — accepted, partial, or
//! rejected — produces a row. See `spec/storage-format.md` "Audit" and
//! `spec/privacy-access.md`.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::ulid::Ulid;
use crate::{Error, Result};

/// Actor type recorded on the audit row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ActorType {
    #[default]
    /// User self-session.
    Self_,
    /// Grant token (includes device tokens).
    Grant,
    /// Storage-internal action.
    System,
    /// Delegate grant token — actor is acting on behalf of another user.
    /// The audit row captures both identities (the delegate's `grant_id`
    /// is `grant_id`; the delegated-for user's ULID is
    /// `delegated_for_user_ulid`).
    Delegate,
}

impl ActorType {
    /// On-disk string form.
    pub fn as_str(self) -> &'static str {
        match self {
            ActorType::Self_ => "self",
            ActorType::Grant => "grant",
            ActorType::System => "system",
            ActorType::Delegate => "delegate",
        }
    }
}

/// Result classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum AuditResult {
    #[default]
    /// Returned what was asked.
    Success,
    /// Some rows or channels stripped silently from the grantee.
    Partial,
    /// Out of scope; nothing returned.
    Rejected,
    /// Internal failure or upstream timeout.
    Error,
}

impl AuditResult {
    /// On-disk string form.
    pub fn as_str(self) -> &'static str {
        match self {
            AuditResult::Success => "success",
            AuditResult::Partial => "partial",
            AuditResult::Rejected => "rejected",
            AuditResult::Error => "error",
        }
    }
}

/// One row in `audit_log`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Time of the operation.
    pub ts_ms: i64,
    /// Actor type.
    pub actor_type: ActorType,
    /// Whether the access fired auto-granted (timeout-default-allow).
    pub auto_granted: bool,
    /// Internal grant id (None for self/system). The wire form uses the ULID;
    /// the audit row stores the rowid for joinability.
    pub grant_id: Option<i64>,
    /// Action label.
    pub action: String,
    /// Sub-classification for reads.
    pub query_kind: Option<String>,
    /// Canonical request payload (Protobuf-JSON).
    pub query_params_json: Option<String>,
    /// Returned rows.
    pub rows_returned: Option<i64>,
    /// Silently filtered rows.
    pub rows_filtered: Option<i64>,
    /// Outcome.
    pub result: AuditResult,
    /// Failure reason.
    pub reason: Option<String>,
    /// Caller IP.
    pub caller_ip: Option<String>,
    /// Caller user-agent.
    pub caller_ua: Option<String>,
    /// For delegate-grant rows: the user being delegated for. Set on every
    /// row written under a delegate token (both the `actor_type='delegate'`
    /// row and the `actor_type='self'` mirror row written for the user's
    /// audit log). NULL on every non-delegate row.
    pub delegated_for_user_ulid: Option<Ulid>,
}

/// Append an audit row. Always called once per OHDC RPC, including for
/// rejections and internal errors.
///
/// For delegate-grant operations, callers should write **two rows**: one
/// with `actor_type=Delegate + grant_id` (the delegate's perspective) and
/// one with `actor_type=Self_ + delegated_for_user_ulid` (the user's
/// perspective). [`append_for_delegate`] is a convenience that does both.
pub fn append(conn: &Connection, e: &AuditEntry) -> Result<()> {
    conn.execute(
        "INSERT INTO audit_log
            (ts_ms, actor_type, auto_granted, grant_id, action, query_kind,
             query_params_json, rows_returned, rows_filtered, result, reason,
             caller_ip, caller_ua, delegated_for_user_ulid)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            e.ts_ms,
            e.actor_type.as_str(),
            e.auto_granted as i64,
            e.grant_id,
            e.action,
            e.query_kind,
            e.query_params_json,
            e.rows_returned,
            e.rows_filtered,
            e.result.as_str(),
            e.reason,
            e.caller_ip,
            e.caller_ua,
            e.delegated_for_user_ulid.map(|u| u.to_vec()),
        ],
    )?;
    Ok(())
}

/// Convenience for delegate-grant audit. Writes two rows: one with
/// `actor_type=Delegate` (caller perspective) and one with
/// `actor_type=Self_` (user perspective, `grant_id=NULL`). Both rows carry
/// `delegated_for_user_ulid` so a query like
/// `SELECT * FROM audit_log WHERE delegated_for_user_ulid = ?` returns the
/// pair.
pub fn append_for_delegate(
    conn: &Connection,
    delegate_grant_id: i64,
    delegated_for_user_ulid: Ulid,
    template: &AuditEntry,
) -> Result<()> {
    // Row 1: the delegate's audit row.
    let mut delegate_row = template.clone();
    delegate_row.actor_type = ActorType::Delegate;
    delegate_row.grant_id = Some(delegate_grant_id);
    delegate_row.delegated_for_user_ulid = Some(delegated_for_user_ulid);
    append(conn, &delegate_row)?;
    // Row 2: the user's mirror — actor='self', grant_id=NULL, but the
    // delegated_for_user_ulid still set so the user sees who acted on
    // their behalf.
    let mut user_row = template.clone();
    user_row.actor_type = ActorType::Self_;
    user_row.grant_id = None;
    user_row.delegated_for_user_ulid = Some(delegated_for_user_ulid);
    user_row.reason = match user_row.reason {
        Some(r) => Some(format!("{r}; via delegate grant {delegate_grant_id}")),
        None => Some(format!("via delegate grant {delegate_grant_id}")),
    };
    append(conn, &user_row)?;
    Ok(())
}

/// Filter for audit queries.
#[derive(Debug, Clone, Default)]
pub struct AuditQuery {
    /// Lower time bound (inclusive).
    pub from_ms: Option<i64>,
    /// Upper time bound (inclusive).
    pub to_ms: Option<i64>,
    /// Filter to a specific grant id.
    pub grant_id: Option<i64>,
    /// Filter by actor type string.
    pub actor_type: Option<String>,
    /// Filter by action string.
    pub action: Option<String>,
    /// Filter by result string.
    pub result: Option<String>,
    /// Limit the number of rows returned.
    pub limit: Option<i64>,
}

/// Run an audit query and return matching rows.
pub fn query(conn: &Connection, q: &AuditQuery) -> Result<Vec<AuditEntry>> {
    let mut sql = String::from(
        "SELECT ts_ms, actor_type, auto_granted, grant_id, action, query_kind,
                query_params_json, rows_returned, rows_filtered, result, reason,
                caller_ip, caller_ua, delegated_for_user_ulid
           FROM audit_log WHERE 1=1",
    );
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(from) = q.from_ms {
        sql.push_str(" AND ts_ms >= ?");
        args.push(from.into());
    }
    if let Some(to) = q.to_ms {
        sql.push_str(" AND ts_ms <= ?");
        args.push(to.into());
    }
    if let Some(gid) = q.grant_id {
        sql.push_str(" AND grant_id = ?");
        args.push(gid.into());
    }
    if let Some(ref a) = q.actor_type {
        sql.push_str(" AND actor_type = ?");
        args.push(a.clone().into());
    }
    if let Some(ref a) = q.action {
        sql.push_str(" AND action = ?");
        args.push(a.clone().into());
    }
    if let Some(ref r) = q.result {
        sql.push_str(" AND result = ?");
        args.push(r.clone().into());
    }
    sql.push_str(" ORDER BY ts_ms DESC");
    if let Some(lim) = q.limit {
        sql.push_str(&format!(" LIMIT {lim}"));
    }
    let mut stmt = conn.prepare(&sql)?;
    let mapped = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |row| {
            let actor: String = row.get(1)?;
            let result: String = row.get(9)?;
            let delegated_blob: Option<Vec<u8>> = row.get(13)?;
            let delegated_for_user_ulid = delegated_blob.and_then(|b| {
                if b.len() == 16 {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b);
                    Some(o)
                } else {
                    None
                }
            });
            Ok(AuditEntry {
                ts_ms: row.get(0)?,
                actor_type: match actor.as_str() {
                    "self" => ActorType::Self_,
                    "grant" => ActorType::Grant,
                    "delegate" => ActorType::Delegate,
                    _ => ActorType::System,
                },
                auto_granted: row.get::<_, i64>(2)? != 0,
                grant_id: row.get(3)?,
                action: row.get(4)?,
                query_kind: row.get(5)?,
                query_params_json: row.get(6)?,
                rows_returned: row.get(7)?,
                rows_filtered: row.get(8)?,
                result: match result.as_str() {
                    "success" => AuditResult::Success,
                    "partial" => AuditResult::Partial,
                    "rejected" => AuditResult::Rejected,
                    _ => AuditResult::Error,
                },
                reason: row.get(10)?,
                caller_ip: row.get(11)?,
                caller_ua: row.get(12)?,
                delegated_for_user_ulid,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(mapped)
}

/// Sweep entries older than `_meta.audit_retention_days`. NULL = forever (no-op).
pub fn sweep_retention(conn: &Connection, now_ms: i64) -> Result<u64> {
    let days: Option<i64> = conn
        .query_row(
            "SELECT value FROM _meta WHERE key='audit_retention_days'",
            [],
            |r| r.get::<_, String>(0),
        )
        .ok()
        .and_then(|s| s.parse().ok());
    let Some(days) = days else { return Ok(0) };
    let cutoff = now_ms - days * 86_400_000;
    let n = conn.execute("DELETE FROM audit_log WHERE ts_ms < ?1", params![cutoff])?;
    Ok(n as u64)
}

/// Convenience for the OHDC layer: stamp `now_ms` from the system clock.
pub fn now_ms() -> i64 {
    crate::format::now_ms()
}

// Keep a one-line use of the Ulid type so `pub use` stays meaningful from the
// consumer side; AuditEntry exposes grant_id by rowid for joinability.
#[allow(dead_code)]
type _UlidIsExposedToConsumers = Ulid;

// Avoid an "unused import" warning if Error becomes only-used-in-cfg path.
#[allow(dead_code)]
fn _unused() -> Result<()> {
    Err(Error::NotFound)
}
