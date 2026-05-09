//! Cases — labelled containers of events with predecessor / parent linkage,
//! lifecycle, and grant-scope expansion.
//!
//! Backs `cases`, `case_filters`, `case_reopen_tokens`, `grant_cases`. See
//! `spec/storage-format.md` "Cases" and `spec/privacy-access.md`
//! "Cases — episodes of care".
//!
//! # Model
//!
//! - A **case** is a row in `cases` with a wire ULID, type, optional label,
//!   start/end timestamps, optional `parent_case_id` (structural rollup), and
//!   optional `predecessor_case_id` (handoff chain).
//! - A case's **scope** is the union of its `case_filters` rows + recursive
//!   union of its predecessor's scope + recursive union of any child case
//!   scopes. See [`compute_case_scope`].
//! - **Grant binding**: rows in `grant_cases` (managed by grants.rs) tie a
//!   grant to one or more cases; the grant's candidate set is the union of
//!   those cases' scopes intersected with the grant's read rules.
//! - **Lifecycle**: open / close / reopen via token / auto-close on inactivity.
//!   Closing issues a [`CaseReopenToken`] with default 24h TTL.
//! - **Read after close**: the case scope on closed cases excludes events with
//!   `events.timestamp_ms > cases.ended_at_ms` so post-close writes never
//!   retroactively land in the case (the read-only-after-close semantic from
//!   `spec/storage-format.md`).
//!
//! # Cycle prevention
//!
//! `parent_case_id` and `predecessor_case_id` chains must form a DAG. Both
//! [`create_case`] and [`update_case`] validate that the chain doesn't loop.

use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::events::EventFilter;
use crate::ulid::{self, Ulid};
use crate::{Error, Result};

/// One case row materialized for the wire.
#[derive(Debug, Clone)]
pub struct Case {
    /// Internal rowid.
    pub id: i64,
    /// Wire ULID (16 bytes; `(started_at_ms, ulid_random)`).
    pub ulid: Ulid,
    /// Type tag, e.g. `"emergency"`, `"admission"`, `"visit"`.
    pub case_type: String,
    /// Optional human-readable label.
    pub case_label: Option<String>,
    /// Start time, Unix ms.
    pub started_at_ms: i64,
    /// Close time; `None` while ongoing.
    pub ended_at_ms: Option<i64>,
    /// Parent case (structural rollup; children → parent).
    pub parent_case_ulid: Option<Ulid>,
    /// Predecessor case (handoff chain; predecessor → successor).
    pub predecessor_case_ulid: Option<Ulid>,
    /// Authority that opened the case (for break-glass / care visits).
    pub opening_authority_grant_ulid: Option<Ulid>,
    /// Inactivity threshold for auto-close, hours. `None` = no auto-close.
    pub inactivity_close_after_h: Option<i32>,
    /// Last activity timestamp; updated on every read/write into the case.
    pub last_activity_at_ms: i64,
}

/// Sparse builder for [`create_case`].
#[derive(Debug, Clone, Default)]
pub struct NewCase {
    /// Type tag (required).
    pub case_type: String,
    /// Optional label.
    pub case_label: Option<String>,
    /// Optional parent case ULID (structural rollup).
    pub parent_case_ulid: Option<Ulid>,
    /// Optional predecessor case ULID (handoff chain).
    pub predecessor_case_ulid: Option<Ulid>,
    /// Optional inactivity threshold in hours.
    pub inactivity_close_after_h: Option<i32>,
    /// Optional initial filters (at least one is recommended; without filters
    /// the case has empty scope until [`add_case_filter`] populates it).
    pub initial_filters: Vec<EventFilter>,
    /// Authority grant ID (rowid) that's opening this case. `None` for
    /// patient-curated cases (self-session).
    pub opening_authority_grant_id: Option<i64>,
}

/// One row in `case_filters`, materialized for the wire.
#[derive(Debug, Clone)]
pub struct CaseFilterRow {
    /// Internal rowid.
    pub id: i64,
    /// Wire ULID (16 bytes; `(added_at_ms, ulid_random)`); minted at insert.
    pub ulid: Ulid,
    /// Owning case rowid.
    pub case_id: i64,
    /// Owning case ULID (for the wire shape).
    pub case_ulid: Ulid,
    /// The filter (decoded from `filter_json`).
    pub filter: EventFilter,
    /// Optional label for the patient-facing audit view.
    pub filter_label: Option<String>,
    /// Insertion timestamp.
    pub added_at_ms: i64,
    /// Authority grant rowid that added the filter (None = patient/self).
    pub added_by_grant_id: Option<i64>,
    /// Soft-delete timestamp; non-null = filter no longer contributes to scope.
    pub removed_at_ms: Option<i64>,
}

/// Reopen token issued when a case auto-closes.
#[derive(Debug, Clone)]
pub struct CaseReopenToken {
    /// Wire ULID; presented by the authority to reopen.
    pub ulid: Ulid,
    /// Owning case ULID.
    pub case_ulid: Ulid,
    /// Authority grant rowid this token belongs to.
    pub authority_grant_id: i64,
    /// Issuance time.
    pub issued_at_ms: i64,
    /// Hard expiry.
    pub expires_at_ms: i64,
}

/// Default reopen-token TTL — 24 hours per spec.
pub const DEFAULT_REOPEN_TTL_MS: i64 = 24 * 60 * 60 * 1000;

// ---------------------------------------------------------------------------
// CRUD
// ---------------------------------------------------------------------------

/// Insert a new case row + its initial filters. Returns `(case_id, case_ulid)`.
///
/// Validates DAG invariants on `parent_case_id` and `predecessor_case_id`.
pub fn create_case(conn: &mut Connection, c: &NewCase) -> Result<(i64, Ulid)> {
    if c.case_type.trim().is_empty() {
        return Err(Error::InvalidArgument("case_type required".into()));
    }
    let now = crate::format::now_ms();
    let new_ulid = ulid::mint(now);
    let rand_tail = ulid::random_tail(&new_ulid);

    let tx = conn.transaction()?;

    // Resolve parent / predecessor ULIDs into rowids and validate DAG.
    let parent_id = match c.parent_case_ulid {
        Some(u) => Some(case_id_by_ulid_in(&tx, &u)?),
        None => None,
    };
    let predecessor_id = match c.predecessor_case_ulid {
        Some(u) => {
            let pid = case_id_by_ulid_in(&tx, &u)?;
            // Validate predecessor is closed OR currently active (it's allowed
            // to chain off an open case; the spec describes both EMS handoff
            // (closes predecessor) and structural references).
            Some(pid)
        }
        None => None,
    };

    tx.execute(
        "INSERT INTO cases
            (ulid_random, case_type, case_label, started_at_ms, ended_at_ms,
             ended_by_grant_id, parent_case_id, predecessor_case_id,
             opening_authority_grant_id, inactivity_close_after_h,
             last_activity_at_ms)
         VALUES (?1, ?2, ?3, ?4, NULL, NULL, ?5, ?6, ?7, ?8, ?9)",
        params![
            rand_tail.to_vec(),
            c.case_type,
            c.case_label,
            now,
            parent_id,
            predecessor_id,
            c.opening_authority_grant_id,
            c.inactivity_close_after_h,
            now,
        ],
    )?;
    let case_id = tx.last_insert_rowid();

    // DAG validation: walk parent + predecessor chains and ensure case_id
    // doesn't appear in either ancestor set.
    validate_dag(&tx, case_id, parent_id, "parent_case_id")?;
    validate_dag(&tx, case_id, predecessor_id, "predecessor_case_id")?;

    // Insert initial filters (audit on each).
    for filter in &c.initial_filters {
        insert_case_filter(&tx, case_id, filter, None, c.opening_authority_grant_id)?;
    }

    tx.commit()?;
    Ok((case_id, new_ulid))
}

/// Sparse update for an existing case. Mutable: label, inactivity threshold.
/// Parent / predecessor are immutable post-creation per spec (handoff opens a
/// new case rather than mutating linkage).
#[derive(Debug, Clone, Default)]
pub struct CaseUpdate {
    /// New label.
    pub case_label: Option<String>,
    /// New inactivity threshold (hours). Pass `Some(0)` to clear.
    pub inactivity_close_after_h: Option<i32>,
}

/// Apply a [`CaseUpdate`]. Returns the updated row.
pub fn update_case(conn: &mut Connection, case_id: i64, update: &CaseUpdate) -> Result<Case> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM cases WHERE id = ?1",
            params![case_id],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(Error::CaseNotFound);
    }
    if let Some(label) = &update.case_label {
        conn.execute(
            "UPDATE cases SET case_label = ?1 WHERE id = ?2",
            params![label, case_id],
        )?;
    }
    if let Some(h) = update.inactivity_close_after_h {
        let v = if h == 0 { None } else { Some(h) };
        conn.execute(
            "UPDATE cases SET inactivity_close_after_h = ?1 WHERE id = ?2",
            params![v, case_id],
        )?;
    }
    read_case(conn, case_id)
}

/// Close a case. Sets `ended_at_ms` if not already set, records the actor
/// grant id, and (when `issue_token` is true and `actor_grant_id` is some)
/// mints a [`CaseReopenToken`] with a 24h default TTL. Idempotent — if the
/// case is already closed, returns the existing case row and no new token.
///
/// Returns `(case, optional_reopen_token)`.
pub fn close_case(
    conn: &mut Connection,
    case_id: i64,
    actor_grant_id: Option<i64>,
    issue_token: bool,
    reopen_ttl_ms: Option<i64>,
) -> Result<(Case, Option<CaseReopenToken>)> {
    let row: Option<(i64, Option<i64>)> = conn
        .query_row(
            "SELECT id, ended_at_ms FROM cases WHERE id = ?1",
            params![case_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((_id, ended_at)) = row else {
        return Err(Error::CaseNotFound);
    };
    if ended_at.is_some() {
        // Already closed — return current state without minting a new token.
        let case = read_case(conn, case_id)?;
        return Ok((case, None));
    }

    let tx = conn.transaction()?;
    let now = crate::format::now_ms();
    tx.execute(
        "UPDATE cases SET ended_at_ms = ?1, ended_by_grant_id = ?2,
                          last_activity_at_ms = ?1
                    WHERE id = ?3",
        params![now, actor_grant_id, case_id],
    )?;

    let reopen_token = if issue_token {
        if let Some(gid) = actor_grant_id {
            let ttl = reopen_ttl_ms.unwrap_or(DEFAULT_REOPEN_TTL_MS);
            let token_ulid = ulid::mint(now);
            let token_tail = ulid::random_tail(&token_ulid);
            let expires = now + ttl;
            tx.execute(
                "INSERT INTO case_reopen_tokens
                    (ulid_random, case_id, authority_grant_id,
                     issued_at_ms, expires_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![token_tail.to_vec(), case_id, gid, now, expires],
            )?;
            let case_ulid = case_ulid_for(&tx, case_id)?;
            Some(CaseReopenToken {
                ulid: token_ulid,
                case_ulid,
                authority_grant_id: gid,
                issued_at_ms: now,
                expires_at_ms: expires,
            })
        } else {
            // Patient-driven close without a grant authority — no reopen token
            // (the patient can re-open via self-session directly without one).
            None
        }
    } else {
        None
    };

    tx.commit()?;
    let case = read_case(conn, case_id)?;
    Ok((case, reopen_token))
}

/// Reopen a closed case by case rowid. Clears `ended_at_ms` and bumps
/// `last_activity_at_ms`. The caller is responsible for any token validation.
pub fn reopen_case(conn: &mut Connection, case_id: i64) -> Result<Case> {
    let now = crate::format::now_ms();
    let n = conn.execute(
        "UPDATE cases SET ended_at_ms = NULL, ended_by_grant_id = NULL,
                          last_activity_at_ms = ?1
                    WHERE id = ?2",
        params![now, case_id],
    )?;
    if n == 0 {
        return Err(Error::CaseNotFound);
    }
    read_case(conn, case_id)
}

/// Look up & redeem a reopen token. Returns the case rowid on success;
/// errors with [`Error::NotFound`] if the token is unknown, [`Error::TokenExpired`]
/// past TTL, [`Error::TokenRevoked`] if revoked or already-used.
pub fn redeem_reopen_token(conn: &mut Connection, token_ulid: &Ulid) -> Result<i64> {
    let rand_tail = ulid::random_tail(token_ulid);
    let now = crate::format::now_ms();

    let tx = conn.transaction()?;
    let row: Option<(i64, i64, i64, Option<i64>, Option<i64>)> = tx
        .query_row(
            "SELECT id, case_id, expires_at_ms, used_at_ms, revoked_at_ms
               FROM case_reopen_tokens WHERE ulid_random = ?1",
            params![rand_tail.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((token_id, case_id, expires_at, used_at, revoked_at)) = row else {
        return Err(Error::NotFound);
    };
    if revoked_at.is_some() || used_at.is_some() {
        return Err(Error::TokenRevoked);
    }
    if expires_at <= now {
        return Err(Error::TokenExpired);
    }
    tx.execute(
        "UPDATE case_reopen_tokens SET used_at_ms = ?1 WHERE id = ?2",
        params![now, token_id],
    )?;
    tx.commit()?;
    Ok(case_id)
}

/// Filter for [`list_cases`].
#[derive(Debug, Clone, Default)]
pub struct ListCasesFilter {
    /// Include closed cases (default false → only ongoing).
    pub include_closed: bool,
    /// Filter by exact case_type.
    pub case_type: Option<String>,
    /// Filter by inclusive lower bound on `started_at_ms`.
    pub from_ms: Option<i64>,
    /// Filter by inclusive upper bound on `started_at_ms`.
    pub to_ms: Option<i64>,
    /// Restrict to cases in this set of rowids — used by grant-scope filtering
    /// when the grant has rows in `grant_cases`. None = unrestricted.
    pub only_case_ids: Option<Vec<i64>>,
    /// Page size (default 100, max 1000).
    pub limit: Option<i64>,
}

/// List cases. Self-session: `only_case_ids = None`. Grant token: caller
/// resolves the grant's `grant_cases` rows and passes them as `only_case_ids`.
pub fn list_cases(conn: &Connection, filter: &ListCasesFilter) -> Result<Vec<Case>> {
    let mut sql = String::from("SELECT id FROM cases WHERE 1=1");
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    if !filter.include_closed {
        sql.push_str(" AND ended_at_ms IS NULL");
    }
    if let Some(ref t) = filter.case_type {
        sql.push_str(" AND case_type = ?");
        args.push(t.clone().into());
    }
    if let Some(from) = filter.from_ms {
        sql.push_str(" AND started_at_ms >= ?");
        args.push(from.into());
    }
    if let Some(to) = filter.to_ms {
        sql.push_str(" AND started_at_ms <= ?");
        args.push(to.into());
    }
    if let Some(ref ids) = filter.only_case_ids {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let placeholders = (0..ids.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        sql.push_str(&format!(" AND id IN ({placeholders})"));
        for id in ids {
            args.push((*id).into());
        }
    }
    sql.push_str(" ORDER BY started_at_ms DESC");
    let limit = filter.limit.unwrap_or(100).clamp(1, 1000);
    sql.push_str(&format!(" LIMIT {limit}"));

    let mut stmt = conn.prepare(&sql)?;
    let ids: Vec<i64> = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(read_case(conn, id)?);
    }
    Ok(out)
}

/// Read one case by rowid.
pub fn read_case(conn: &Connection, case_id: i64) -> Result<Case> {
    let row: Option<(
        Vec<u8>,
        String,
        Option<String>,
        i64,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i64>,
        Option<i32>,
        i64,
    )> = conn
        .query_row(
            "SELECT ulid_random, case_type, case_label, started_at_ms,
                    ended_at_ms, parent_case_id, predecessor_case_id,
                    opening_authority_grant_id, inactivity_close_after_h,
                    last_activity_at_ms
               FROM cases WHERE id = ?1",
            params![case_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                ))
            },
        )
        .optional()?;
    let Some((
        rand_tail,
        case_type,
        case_label,
        started_at_ms,
        ended_at_ms,
        parent_case_id,
        predecessor_case_id,
        opening_authority_grant_id,
        inactivity_close_after_h,
        last_activity_at_ms,
    )) = row
    else {
        return Err(Error::CaseNotFound);
    };
    let ulid = ulid::from_parts(started_at_ms, &rand_tail)?;
    let parent_case_ulid = match parent_case_id {
        Some(id) => Some(case_ulid_for(conn, id)?),
        None => None,
    };
    let predecessor_case_ulid = match predecessor_case_id {
        Some(id) => Some(case_ulid_for(conn, id)?),
        None => None,
    };
    let opening_authority_grant_ulid = match opening_authority_grant_id {
        Some(id) => grant_ulid_for(conn, id).ok(),
        None => None,
    };
    Ok(Case {
        id: case_id,
        ulid,
        case_type,
        case_label,
        started_at_ms,
        ended_at_ms,
        parent_case_ulid,
        predecessor_case_ulid,
        opening_authority_grant_ulid,
        inactivity_close_after_h,
        last_activity_at_ms,
    })
}

/// Look up a case rowid by its wire ULID.
pub fn case_id_by_ulid(conn: &Connection, case_ulid: &Ulid) -> Result<i64> {
    let rand_tail = ulid::random_tail(case_ulid);
    conn.query_row(
        "SELECT id FROM cases WHERE ulid_random = ?1",
        params![rand_tail.to_vec()],
        |r| r.get::<_, i64>(0),
    )
    .optional()?
    .ok_or(Error::CaseNotFound)
}

fn case_id_by_ulid_in(tx: &Transaction<'_>, case_ulid: &Ulid) -> Result<i64> {
    let rand_tail = ulid::random_tail(case_ulid);
    tx.query_row(
        "SELECT id FROM cases WHERE ulid_random = ?1",
        params![rand_tail.to_vec()],
        |r| r.get::<_, i64>(0),
    )
    .optional()?
    .ok_or(Error::CaseNotFound)
}

fn case_ulid_for(conn: &Connection, case_id: i64) -> Result<Ulid> {
    let row: Option<(Vec<u8>, i64)> = conn
        .query_row(
            "SELECT ulid_random, started_at_ms FROM cases WHERE id = ?1",
            params![case_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((rt, ts)) = row else {
        return Err(Error::CaseNotFound);
    };
    ulid::from_parts(ts, &rt)
}

fn grant_ulid_for(conn: &Connection, grant_id: i64) -> Result<Ulid> {
    let row: Option<(Vec<u8>, i64)> = conn
        .query_row(
            "SELECT ulid_random, created_at_ms FROM grants WHERE id = ?1",
            params![grant_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((rt, ts)) = row else {
        return Err(Error::NotFound);
    };
    ulid::from_parts(ts, &rt)
}

// ---------------------------------------------------------------------------
// Case filters
// ---------------------------------------------------------------------------

/// Add a filter to a case. Returns the new [`CaseFilterRow`].
pub fn add_case_filter(
    conn: &mut Connection,
    case_id: i64,
    filter: &EventFilter,
    filter_label: Option<&str>,
    added_by_grant_id: Option<i64>,
) -> Result<CaseFilterRow> {
    let exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM cases WHERE id = ?1",
            params![case_id],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(Error::CaseNotFound);
    }
    let tx = conn.transaction()?;
    let id = insert_case_filter(&tx, case_id, filter, filter_label, added_by_grant_id)?;
    tx.commit()?;
    read_case_filter(conn, id)
}

fn insert_case_filter(
    tx: &Transaction<'_>,
    case_id: i64,
    filter: &EventFilter,
    filter_label: Option<&str>,
    added_by_grant_id: Option<i64>,
) -> Result<i64> {
    let payload = serde_json::to_string(filter)?;
    let now = crate::format::now_ms();
    tx.execute(
        "INSERT INTO case_filters
            (case_id, filter_json, filter_label, added_at_ms, added_by_grant_id)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![case_id, payload, filter_label, now, added_by_grant_id],
    )?;
    Ok(tx.last_insert_rowid())
}

/// Soft-delete a case filter (sets `removed_at_ms` to now). Idempotent.
pub fn remove_case_filter(conn: &Connection, filter_id: i64) -> Result<i64> {
    let now = crate::format::now_ms();
    let n = conn.execute(
        "UPDATE case_filters SET removed_at_ms = ?1
          WHERE id = ?2 AND removed_at_ms IS NULL",
        params![now, filter_id],
    )?;
    if n == 0 {
        // Either filter not found, or already removed; verify which.
        let exists: Option<i64> = conn
            .query_row(
                "SELECT id FROM case_filters WHERE id = ?1",
                params![filter_id],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_none() {
            return Err(Error::NotFound);
        }
    }
    Ok(now)
}

/// List a case's filters.
pub fn list_case_filters(
    conn: &Connection,
    case_id: i64,
    include_removed: bool,
) -> Result<Vec<CaseFilterRow>> {
    let mut sql = String::from("SELECT id FROM case_filters WHERE case_id = ?1");
    if !include_removed {
        sql.push_str(" AND removed_at_ms IS NULL");
    }
    sql.push_str(" ORDER BY added_at_ms ASC");
    let mut stmt = conn.prepare(&sql)?;
    let ids: Vec<i64> = stmt
        .query_map(params![case_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        out.push(read_case_filter(conn, id)?);
    }
    Ok(out)
}

/// Read one filter by rowid.
pub fn read_case_filter(conn: &Connection, filter_id: i64) -> Result<CaseFilterRow> {
    let row: Option<(i64, String, Option<String>, i64, Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT case_id, filter_json, filter_label, added_at_ms,
                    added_by_grant_id, removed_at_ms
               FROM case_filters WHERE id = ?1",
            params![filter_id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()?;
    let Some((case_id, filter_json, filter_label, added_at_ms, added_by_grant_id, removed_at_ms)) =
        row
    else {
        return Err(Error::NotFound);
    };
    let filter: EventFilter = serde_json::from_str(&filter_json)?;
    let case_ulid = case_ulid_for(conn, case_id)?;
    // Filter ULIDs aren't currently persisted as a column — they're derived
    // from `(added_at_ms, filter_id)` for wire stability. v1.x can promote a
    // dedicated `ulid_random` column when filter ULIDs need to round-trip
    // across deployments; for now we mint deterministically from the rowid.
    let mut rand_tail = [0u8; 10];
    rand_tail[..8].copy_from_slice(&filter_id.to_be_bytes());
    let ulid = ulid::from_parts(added_at_ms, &rand_tail)?;
    Ok(CaseFilterRow {
        id: filter_id,
        ulid,
        case_id,
        case_ulid,
        filter,
        filter_label,
        added_at_ms,
        added_by_grant_id,
        removed_at_ms,
    })
}

// ---------------------------------------------------------------------------
// Scope resolution (predecessor + child traversal)
// ---------------------------------------------------------------------------

/// Compute the case's effective scope: a list of `EventFilter`s whose union
/// matches every event in the case's recursive scope.
///
/// Returns the OR-merge of:
/// 1. The case's own non-removed `case_filters`.
/// 2. The recursive `case_scope` of its predecessor (forward inheritance).
/// 3. The recursive `case_scope` of every child case
///    (`parent_case_id = self.id`).
///
/// Each filter is clamped to `events.timestamp_ms <= ended_at_ms` for closed
/// cases (the read-only-after-close semantic from `spec/storage-format.md`).
///
/// Cycle-safe via a rowid set. v1 returns the filter list; the events module
/// OR-merges by issuing one query per filter and de-duplicating by ULID.
pub fn compute_case_scope(conn: &Connection, case_id: i64) -> Result<Vec<EventFilter>> {
    let mut visited: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut out: Vec<EventFilter> = Vec::new();
    walk_scope(conn, case_id, &mut visited, &mut out)?;
    Ok(out)
}

fn walk_scope(
    conn: &Connection,
    case_id: i64,
    visited: &mut std::collections::HashSet<i64>,
    out: &mut Vec<EventFilter>,
) -> Result<()> {
    if !visited.insert(case_id) {
        return Ok(());
    }
    // Read this case's metadata.
    let row: Option<(Option<i64>, Option<i64>)> = conn
        .query_row(
            "SELECT ended_at_ms, predecessor_case_id FROM cases WHERE id = ?1",
            params![case_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let Some((ended_at_ms, predecessor_case_id)) = row else {
        return Ok(());
    };
    // Append this case's own filters, clamped to ended_at_ms when closed.
    let mut stmt = conn.prepare(
        "SELECT filter_json FROM case_filters
          WHERE case_id = ?1 AND removed_at_ms IS NULL",
    )?;
    let mut rows = stmt.query(params![case_id])?;
    while let Some(row) = rows.next()? {
        let payload: String = row.get(0)?;
        let mut f: EventFilter = serde_json::from_str(&payload)?;
        if let Some(end) = ended_at_ms {
            // Clamp the filter's upper bound to the close timestamp so post-close
            // writes never retroactively land in this case.
            f.to_ms = Some(match f.to_ms {
                Some(t) => t.min(end),
                None => end,
            });
        }
        out.push(f);
    }
    drop(rows);
    drop(stmt);

    // Predecessor inheritance (forward): successor sees predecessor's scope.
    if let Some(pid) = predecessor_case_id {
        walk_scope(conn, pid, visited, out)?;
    }

    // Child rollup: parent sees children's scopes (children → parent direction).
    let mut stmt = conn.prepare("SELECT id FROM cases WHERE parent_case_id = ?1")?;
    let child_ids: Vec<i64> = stmt
        .query_map(params![case_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(stmt);
    for cid in child_ids {
        walk_scope(conn, cid, visited, out)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Grant-case binding helpers
// ---------------------------------------------------------------------------

/// List the case rowids a grant is bound to via `grant_cases`. Empty result
/// = open-scope grant (no case binding).
pub fn grant_case_ids(conn: &Connection, grant_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare("SELECT case_id FROM grant_cases WHERE grant_id = ?1")?;
    let ids: Vec<i64> = stmt
        .query_map(params![grant_id], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(ids)
}

/// Bind a grant to one or more cases. Idempotent (uses INSERT OR IGNORE).
pub fn bind_grant_to_cases(conn: &Connection, grant_id: i64, case_ids: &[i64]) -> Result<()> {
    let now = crate::format::now_ms();
    for cid in case_ids {
        conn.execute(
            "INSERT OR IGNORE INTO grant_cases (grant_id, case_id, added_at_ms)
             VALUES (?1, ?2, ?3)",
            params![grant_id, cid, now],
        )?;
    }
    Ok(())
}

/// Update the case's `last_activity_at_ms` to `now_ms`. Called on each
/// read/write that resolves into the case.
pub fn touch_activity(conn: &Connection, case_id: i64) -> Result<()> {
    let now = crate::format::now_ms();
    conn.execute(
        "UPDATE cases SET last_activity_at_ms = ?1 WHERE id = ?2",
        params![now, case_id],
    )?;
    Ok(())
}

// ---------------------------------------------------------------------------
// DAG validation
// ---------------------------------------------------------------------------

/// Walk a case's ancestor chain through `column` and ensure `case_id` doesn't
/// reappear (cycle).
fn validate_dag(
    tx: &Transaction<'_>,
    case_id: i64,
    seed_ancestor: Option<i64>,
    column: &str,
) -> Result<()> {
    let Some(mut cur) = seed_ancestor else {
        return Ok(());
    };
    let mut visited: std::collections::HashSet<i64> = std::collections::HashSet::new();
    visited.insert(case_id);
    loop {
        if !visited.insert(cur) {
            return Err(Error::InvalidArgument(format!(
                "cycle detected in {column} chain at case {cur}"
            )));
        }
        let sql = format!("SELECT {column} FROM cases WHERE id = ?1");
        let next: Option<Option<i64>> =
            tx.query_row(&sql, params![cur], |r| r.get(0)).optional()?;
        match next {
            Some(Some(parent)) => cur = parent,
            _ => return Ok(()),
        }
    }
}

// ---------------------------------------------------------------------------
// Sweep
// ---------------------------------------------------------------------------

/// Auto-close cases whose `last_activity_at_ms` is older than their
/// `inactivity_close_after_h` threshold. Returns `(case_ids_closed,
/// reopen_tokens_issued)`.
///
/// The server should call this from a periodic tokio task. For each closed
/// case where the closing authority has `opening_authority_grant_id` set, a
/// reopen token is also issued (default 24h TTL) so the authority can resume
/// without re-running the break-glass / patient-approval flow.
pub fn sweep_inactive(conn: &mut Connection) -> Result<(Vec<i64>, Vec<CaseReopenToken>)> {
    let now = crate::format::now_ms();
    // Find open cases whose last activity exceeds their threshold.
    let candidates: Vec<(i64, Option<i64>, i32)> = {
        let mut stmt = conn.prepare(
            "SELECT id, opening_authority_grant_id, inactivity_close_after_h
               FROM cases
              WHERE ended_at_ms IS NULL
                AND inactivity_close_after_h IS NOT NULL
                AND last_activity_at_ms + inactivity_close_after_h * 3600000 <= ?1",
        )?;
        let rows: Vec<(i64, Option<i64>, i32)> = stmt
            .query_map(params![now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows
    };

    let mut closed = Vec::new();
    let mut tokens = Vec::new();
    for (case_id, opening_grant, _h) in candidates {
        let (_case, token) = close_case(conn, case_id, opening_grant, true, None)?;
        closed.push(case_id);
        if let Some(t) = token {
            tokens.push(t);
        }
    }
    Ok((closed, tokens))
}
