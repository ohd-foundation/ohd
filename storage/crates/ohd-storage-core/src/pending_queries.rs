//! Per-query approval queue (`grants.require_approval_per_query`).
//!
//! Backs `pending_queries`. Sister of [`crate::pending`] (which is the
//! write-with-approval queue): writes carry a payload to commit on approval;
//! reads carry a *query* to execute on approval. The shapes don't overlap
//! enough to warrant fusion.
//!
//! See `spec/storage-format.md` "Privacy and access control" + the
//! `require_approval_per_query` policy section in `spec/privacy-access.md`.
//! Operationally:
//!
//! 1. Grant token issues a read RPC (e.g. `QueryEvents`).
//! 2. Storage finds `grants.require_approval_per_query=1` on the grant;
//!    instead of executing the query, it inserts a `pending_queries` row
//!    and returns `PendingApproval { query_ulid, expires_at_ms }`.
//! 3. The user's Connect app (over a self-session token) calls
//!    [`list_pending_queries`] / [`approve_pending_query`] / [`reject_pending_query`].
//! 4. On approval, the grantee retries the same RPC; storage looks up the
//!    canonical query (by hash), sees `decision='approved'`, and executes
//!    the query.
//!
//! The queue is exposed both through in-process helpers and the
//! `OhdcService.{List,Approve,Reject}PendingQuery` RPCs.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ulid::{self, Ulid};
use crate::{Error, Result};

/// Decision lifecycle for a pending query row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueryDecision {
    /// Awaiting user review.
    Pending,
    /// User approved; the grantee may now re-run the same query.
    Approved,
    /// User rejected; subsequent re-runs return `OUT_OF_SCOPE`.
    Rejected,
    /// Auto-expired before review.
    Expired,
}

impl QueryDecision {
    /// On-disk string form.
    pub fn as_str(self) -> &'static str {
        match self {
            QueryDecision::Pending => "pending",
            QueryDecision::Approved => "approved",
            QueryDecision::Rejected => "rejected",
            QueryDecision::Expired => "expired",
        }
    }

    /// Parse the string form.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(QueryDecision::Pending),
            "approved" => Ok(QueryDecision::Approved),
            "rejected" => Ok(QueryDecision::Rejected),
            "expired" => Ok(QueryDecision::Expired),
            other => Err(Error::InvalidArgument(format!(
                "unknown pending-query decision {other:?}"
            ))),
        }
    }
}

/// Materialized `pending_queries` row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingQueryRow {
    /// Wire ULID for this query (Crockford-base32 representation).
    pub ulid: String,
    /// Internal grant rowid the query was issued under.
    pub grant_id: i64,
    /// `query_kind` — one of `query_events`, `aggregate`, `correlate`,
    /// `read_samples`, `read_attachment`, `get_event_by_ulid`.
    pub query_kind: String,
    /// Hex-encoded sha256 of the canonical payload (used for re-run lookup).
    pub query_hash_hex: String,
    /// Canonical JSON payload of the original request.
    pub query_payload: String,
    /// Submission time (Unix ms).
    pub requested_at_ms: i64,
    /// Auto-expiry (Unix ms).
    pub expires_at_ms: i64,
    /// Review time (Unix ms), once decided.
    pub decided_at_ms: Option<i64>,
    /// Lifecycle decision.
    pub decision: QueryDecision,
}

/// Default auto-expiry for pending queries: 24 hours.
pub const DEFAULT_TTL_MS: i64 = 24 * 3600 * 1000;

/// Insert a new `pending_queries` row for `(grant_id, query_kind, payload_json)`.
/// Returns the wire ULID + the auto-expiry.
///
/// `payload_json` should be a canonical JSON encoding of the request — same
/// canonicalization used for `audit_log.query_params_json` so the same query
/// re-issued by the grantee produces the same hash.
pub fn enqueue(
    conn: &Connection,
    grant_id: i64,
    query_kind: &str,
    payload_json: &str,
    ttl_ms: Option<i64>,
) -> Result<(Ulid, i64)> {
    let now = crate::format::now_ms();
    let ttl = ttl_ms.unwrap_or(DEFAULT_TTL_MS);
    let expires_at = now.saturating_add(ttl);
    let new_ulid = ulid::mint(now);
    let rand_tail = ulid::random_tail(&new_ulid);
    let mut hasher = Sha256::new();
    hasher.update(query_kind.as_bytes());
    hasher.update(b"\x00");
    hasher.update(payload_json.as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();
    conn.execute(
        "INSERT INTO pending_queries
            (ulid_random, grant_id, query_kind, query_hash, query_payload,
             requested_at_ms, expires_at_ms, decision)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending')",
        params![
            rand_tail.to_vec(),
            grant_id,
            query_kind,
            hash.to_vec(),
            payload_json,
            now,
            expires_at,
        ],
    )?;
    Ok((new_ulid, expires_at))
}

/// Look up the most recent pending-query row for `(grant_id, query_kind, payload_json)`.
///
/// Returns:
/// - `Some(QueryDecision::Approved)` — grantee may proceed with the query.
/// - `Some(QueryDecision::Pending)` — still awaiting review.
/// - `Some(QueryDecision::Rejected)` — user said no; OHDC layer maps to `OUT_OF_SCOPE`.
/// - `Some(QueryDecision::Expired)` — auto-expired; OHDC layer maps to
///   `APPROVAL_TIMEOUT`.
/// - `None` — no row matches; caller should `enqueue()` and return
///   `PENDING_APPROVAL`.
///
/// The most recent row matching the hash wins — the user can re-approve a
/// previously-rejected query (re-issuing creates a new row).
pub fn lookup_decision(
    conn: &Connection,
    grant_id: i64,
    query_kind: &str,
    payload_json: &str,
) -> Result<Option<(Ulid, QueryDecision, i64)>> {
    let mut hasher = Sha256::new();
    hasher.update(query_kind.as_bytes());
    hasher.update(b"\x00");
    hasher.update(payload_json.as_bytes());
    let hash: [u8; 32] = hasher.finalize().into();
    let row: Option<(Vec<u8>, String, i64, i64)> = conn
        .query_row(
            "SELECT ulid_random, decision, expires_at_ms, requested_at_ms
               FROM pending_queries
              WHERE grant_id = ?1 AND query_kind = ?2 AND query_hash = ?3
              ORDER BY id DESC
              LIMIT 1",
            params![grant_id, query_kind, hash.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((rand_tail, decision_s, expires_at_ms, requested_at_ms)) = row else {
        return Ok(None);
    };
    let mut decision = QueryDecision::parse(&decision_s)?;
    let now = crate::format::now_ms();
    if decision == QueryDecision::Pending && now > expires_at_ms {
        decision = QueryDecision::Expired;
    }
    let ulid = ulid::from_parts(requested_at_ms, &rand_tail)?;
    Ok(Some((ulid, decision, expires_at_ms)))
}

/// List pending-query rows. Self-session callers see all rows; pass a
/// `grant_id` filter to restrict.
#[derive(Debug, Clone, Default)]
pub struct ListPendingQueriesFilter {
    /// Restrict by grant rowid.
    pub grant_id: Option<i64>,
    /// Restrict by decision.
    pub decision: Option<QueryDecision>,
    /// Restrict to rows requested at or after this Unix ms timestamp.
    pub since_ms: Option<i64>,
    /// Page size (default 100, max 1000).
    pub limit: Option<i64>,
}

/// Return rows matching `filter` ordered by submission time DESC.
pub fn list_pending_queries(
    conn: &Connection,
    filter: &ListPendingQueriesFilter,
) -> Result<Vec<PendingQueryRow>> {
    let mut sql = String::from(
        "SELECT ulid_random, grant_id, query_kind, query_hash, query_payload,
                requested_at_ms, expires_at_ms, decided_at_ms, decision
           FROM pending_queries WHERE 1=1",
    );
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(gid) = filter.grant_id {
        sql.push_str(" AND grant_id = ?");
        args.push(gid.into());
    }
    if let Some(d) = filter.decision {
        sql.push_str(" AND decision = ?");
        args.push(d.as_str().to_string().into());
    }
    if let Some(since_ms) = filter.since_ms {
        sql.push_str(" AND requested_at_ms >= ?");
        args.push(since_ms.into());
    }
    sql.push_str(" ORDER BY requested_at_ms DESC");
    let limit = filter.limit.unwrap_or(100).clamp(1, 1000);
    sql.push_str(&format!(" LIMIT {limit}"));
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Vec<u8>>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, Option<i64>>(7)?,
                r.get::<_, String>(8)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (
        rand_tail,
        grant_id,
        query_kind,
        query_hash,
        query_payload,
        requested_at_ms,
        expires_at_ms,
        decided_at_ms,
        decision_s,
    ) in rows
    {
        let ulid = ulid::from_parts(requested_at_ms, &rand_tail)?;
        out.push(PendingQueryRow {
            ulid: ulid::to_crockford(&ulid),
            grant_id,
            query_kind,
            query_hash_hex: hex::encode(query_hash),
            query_payload,
            requested_at_ms,
            expires_at_ms,
            decided_at_ms,
            decision: QueryDecision::parse(&decision_s)?,
        });
    }
    Ok(out)
}

/// Approve a pending-query row by its wire ULID.
///
/// Marks `decision='approved'`. Subsequent calls to [`lookup_decision`] for
/// the same `(grant_id, query_kind, payload_json)` will return `Approved`.
/// Returns the row's `decided_at_ms`.
pub fn approve(conn: &Connection, query_ulid: &Ulid) -> Result<i64> {
    transition(conn, query_ulid, QueryDecision::Approved)
}

/// Reject a pending-query row by its wire ULID.
pub fn reject(conn: &Connection, query_ulid: &Ulid) -> Result<i64> {
    transition(conn, query_ulid, QueryDecision::Rejected)
}

fn transition(conn: &Connection, query_ulid: &Ulid, new_decision: QueryDecision) -> Result<i64> {
    let rand_tail = ulid::random_tail(query_ulid);
    let now = crate::format::now_ms();
    let row: Option<(i64, String, i64)> = conn
        .query_row(
            "SELECT id, decision, expires_at_ms
               FROM pending_queries WHERE ulid_random = ?1",
            params![rand_tail.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let Some((id, current, expires_at_ms)) = row else {
        return Err(Error::NotFound);
    };
    if current != "pending" {
        return Err(Error::InvalidArgument(format!(
            "pending_queries row in state {current:?}, not 'pending'"
        )));
    }
    if now > expires_at_ms {
        // Treat as expired even if we're trying to approve; OHDC maps this
        // to APPROVAL_TIMEOUT for the grantee re-run.
        conn.execute(
            "UPDATE pending_queries SET decision = 'expired', decided_at_ms = ?1
              WHERE id = ?2",
            params![now, id],
        )?;
        return Err(Error::ApprovalTimeout);
    }
    conn.execute(
        "UPDATE pending_queries SET decision = ?1, decided_at_ms = ?2 WHERE id = ?3",
        params![new_decision.as_str(), now, id],
    )?;
    Ok(now)
}

/// Sweep pending rows whose `expires_at_ms` is in the past — flips them to
/// `expired`. Returns the number of rows touched. Operators run this on a
/// timer; v1 doesn't yet wire a daemon (mirrors `pending::sweep_expired`).
pub fn sweep_expired(conn: &Connection, now_ms: i64) -> Result<u64> {
    let n = conn.execute(
        "UPDATE pending_queries
            SET decision = 'expired', decided_at_ms = ?1
          WHERE decision = 'pending' AND expires_at_ms <= ?1",
        params![now_ms],
    )?;
    Ok(n as u64)
}
