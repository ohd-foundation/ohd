//! Pending events — write-with-approval queue.
//!
//! Backs `pending_events`. Inbound queueing is driven from
//! [`crate::events::put_events`]; this module owns the read side
//! (list / approve / reject / expire-sweep). See `spec/storage-format.md`
//! "Write-with-approval" and `spec/ohdc-protocol.md` "Pending events".

use rusqlite::{params, Connection};

use crate::audit::{self, AuditEntry, AuditResult};
use crate::events::{ChannelScalar, ChannelValue, Event, EventInput};
use crate::registry;
use crate::ulid::{self, Ulid};
use crate::{Error, Result};

/// Status of a pending event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingStatus {
    /// Awaiting review.
    Pending,
    /// User approved; promoted to `events`.
    Approved,
    /// User rejected.
    Rejected,
    /// Auto-expired before review.
    Expired,
}

impl PendingStatus {
    /// On-disk string form.
    pub fn as_str(self) -> &'static str {
        match self {
            PendingStatus::Pending => "pending",
            PendingStatus::Approved => "approved",
            PendingStatus::Rejected => "rejected",
            PendingStatus::Expired => "expired",
        }
    }
}

/// One row in `pending_events`, materialized for the wire layer.
#[derive(Debug, Clone)]
pub struct PendingRow {
    /// Event ULID (Crockford form mapping to `(timestamp_ms, ulid_random)`).
    pub ulid: Ulid,
    /// Submission time.
    pub submitted_at_ms: i64,
    /// Submitting grant rowid.
    pub submitting_grant_id: i64,
    /// Submitting grant ULID, if resolvable.
    pub submitting_grant_ulid: Option<Ulid>,
    /// Materialized event (decoded from `payload_json`).
    pub event: Event,
    /// Lifecycle status.
    pub status: PendingStatus,
    /// Review time.
    pub reviewed_at_ms: Option<i64>,
    /// Optional rejection reason.
    pub rejection_reason: Option<String>,
    /// Auto-expiry.
    pub expires_at_ms: i64,
    /// If approved, the canonical event's ULID (always equal to `ulid` here).
    pub approved_event_ulid: Option<Ulid>,
}

/// Filter for [`list_pending`].
#[derive(Debug, Clone, Default)]
pub struct ListPendingFilter {
    /// Restrict to a specific submitting grant rowid (for grant-token callers).
    pub submitting_grant_id: Option<i64>,
    /// Restrict by status.
    pub status: Option<PendingStatus>,
    /// Page size (default 100, max 1000).
    pub limit: Option<i64>,
}

/// List pending events. Self-session callers see all rows; grant-token callers
/// pass their own `grant_id` via `filter.submitting_grant_id` so they only see
/// their own submissions (introspection of in-flight writes).
pub fn list_pending(conn: &Connection, filter: &ListPendingFilter) -> Result<Vec<PendingRow>> {
    let mut sql = String::from(
        "SELECT p.ulid_random, p.submitted_at_ms, p.submitting_grant_id, p.payload_json,
                p.status, p.reviewed_at_ms, p.rejection_reason, p.expires_at_ms,
                p.approved_event_id, g.ulid_random
           FROM pending_events p
           LEFT JOIN grants g ON g.id = p.submitting_grant_id
          WHERE 1=1",
    );
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    if let Some(gid) = filter.submitting_grant_id {
        sql.push_str(" AND p.submitting_grant_id = ?");
        args.push(gid.into());
    }
    if let Some(st) = filter.status {
        sql.push_str(" AND p.status = ?");
        args.push(st.as_str().to_string().into());
    }
    sql.push_str(" ORDER BY p.submitted_at_ms DESC");
    let limit = filter.limit.unwrap_or(100).clamp(1, 1000);
    sql.push_str(&format!(" LIMIT {limit}"));

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, Option<i64>>(5)?,
                r.get::<_, Option<String>>(6)?,
                r.get::<_, i64>(7)?,
                r.get::<_, Option<i64>>(8)?,
                r.get::<_, Option<Vec<u8>>>(9)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::with_capacity(rows.len());
    for (
        rand_tail,
        submitted_at,
        grant_id,
        payload_json,
        status_s,
        reviewed_at,
        reject_reason,
        expires_at,
        approved_event_id,
        grant_rand_tail,
    ) in rows
    {
        let status = parse_status(&status_s)?;
        let input: EventInput = serde_json::from_str(&payload_json)?;
        let event_ulid = ulid::from_parts(input.timestamp_ms, &rand_tail)?;
        let event = event_from_input(input.clone(), &event_ulid);
        let submitting_grant_ulid = grant_rand_tail
            .as_deref()
            .and_then(|t| (t.len() == 10).then(|| stitch_grant_ulid(t)));
        let approved_event_ulid = match approved_event_id {
            Some(eid) => fetch_event_ulid_by_rowid(conn, eid)?.into(),
            None => None,
        };
        out.push(PendingRow {
            ulid: event_ulid,
            submitted_at_ms: submitted_at,
            submitting_grant_id: grant_id,
            submitting_grant_ulid,
            event,
            status,
            reviewed_at_ms: reviewed_at,
            rejection_reason: reject_reason,
            expires_at_ms: expires_at,
            approved_event_ulid,
        });
    }
    Ok(out)
}

/// Approve a pending event by its ULID. Promotes the row into `events` +
/// `event_channels`, preserving the ULID; sets `pending.status='approved'`
/// and writes audit. Idempotent only when the row is in `pending` state.
///
/// `also_auto_approve_this_type`: when true, adds the event's type to the
/// submitting grant's `grant_auto_approve_event_types` so future writes of
/// the same type from this grant skip the queue.
///
/// Returns `(committed_at_ms, event_ulid)`.
pub fn approve_pending(
    conn: &mut Connection,
    pending_ulid: &Ulid,
    also_auto_approve_this_type: bool,
    envelope_key: Option<&crate::encryption::EnvelopeKey>,
) -> Result<(i64, Ulid)> {
    let rand_tail = ulid::random_tail(pending_ulid);
    let tx = conn.transaction()?;

    let row: Option<(i64, i64, i64, String, String)> = {
        let mut stmt = tx.prepare(
            "SELECT id, submitted_at_ms, submitting_grant_id, status, payload_json
               FROM pending_events WHERE ulid_random = ?1",
        )?;
        stmt.query_row(params![rand_tail.to_vec()], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })
        .ok()
    };
    let Some((pending_id, _submitted_at_ms, grant_id, status, payload_json)) = row else {
        return Err(Error::NotFound);
    };
    if status != "pending" {
        return Err(Error::InvalidArgument(format!(
            "pending row is in status {status:?}, not 'pending'"
        )));
    }

    let input: EventInput = serde_json::from_str(&payload_json)?;
    let etn = registry::EventTypeName::parse(&input.event_type)?;
    let etype = registry::resolve_event_type(&tx, &etn)?;

    tx.execute(
        "INSERT INTO events
            (ulid_random, timestamp_ms, tz_offset_minutes, tz_name, duration_ms,
             event_type_id, source, source_id, notes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            rand_tail.to_vec(),
            input.timestamp_ms,
            input.tz_offset_minutes,
            input.tz_name,
            input.duration_ms,
            etype.id,
            input.source,
            input.source_id,
            input.notes,
        ],
    )?;
    let event_id = tx.last_insert_rowid();

    for cv in &input.channels {
        let chan = registry::resolve_channel(&tx, etype.id, &cv.channel_path)?;

        // Same encryption gate as `events::insert_channel_value` — if the
        // channel's `sensitivity_class` is in the encrypted-classes set and we
        // have an envelope key, write the value as an AEAD blob; otherwise
        // fall through to the plaintext columns.
        if crate::encryption::is_encrypted_class(&chan.sensitivity_class) {
            if let Some(env) = envelope_key {
                let active =
                    crate::encryption::load_active_class_key_tx(&tx, env, &chan.sensitivity_class)?;
                // Codex review #1+#2: XChaCha20-Poly1305 with wide AAD.
                let blob = crate::channel_encryption::encrypt_channel_value(
                    &chan.path,
                    &cv.value,
                    &active.key,
                    pending_ulid,
                    active.key_id,
                )?;
                tx.execute(
                    "INSERT INTO event_channels
                        (event_id, channel_id, encrypted, value_blob, encryption_key_id)
                     VALUES (?1, ?2, 1, ?3, ?4)",
                    params![event_id, chan.id, blob.to_bytes(), active.key_id,],
                )?;
                continue;
            } else {
                tracing::warn!(
                    channel_path = %chan.path,
                    sensitivity_class = %chan.sensitivity_class,
                    "approve_pending: writing encrypted-class channel as plaintext (no envelope key)"
                );
            }
        }

        let (vr, vi, vt, ve): (Option<f64>, Option<i64>, Option<String>, Option<i32>) = match &cv
            .value
        {
            ChannelScalar::Real { real_value } => (Some(*real_value), None, None, None),
            ChannelScalar::Int { int_value } => (None, Some(*int_value), None, None),
            ChannelScalar::Bool { bool_value } => (None, Some(*bool_value as i64), None, None),
            ChannelScalar::Text { text_value } => (None, None, Some(text_value.clone()), None),
            ChannelScalar::EnumOrdinal { enum_ordinal } => (None, None, None, Some(*enum_ordinal)),
        };
        tx.execute(
            "INSERT INTO event_channels
                (event_id, channel_id, value_real, value_int, value_text, value_enum, encrypted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
            params![event_id, chan.id, vr, vi, vt, ve],
        )?;
    }

    let now = crate::format::now_ms();
    tx.execute(
        "UPDATE pending_events
            SET status = 'approved', reviewed_at_ms = ?1, approved_event_id = ?2
          WHERE id = ?3",
        params![now, event_id, pending_id],
    )?;

    if also_auto_approve_this_type {
        tx.execute(
            "INSERT OR IGNORE INTO grant_auto_approve_event_types (grant_id, event_type_id)
             VALUES (?1, ?2)",
            params![grant_id, etype.id],
        )?;
    }

    audit::append(
        &tx,
        &AuditEntry {
            ts_ms: now,
            actor_type: audit::ActorType::Self_,
            auto_granted: false,
            grant_id: Some(grant_id),
            action: "pending_approve".into(),
            query_kind: Some("approve_pending".into()),
            query_params_json: Some(format!(
                "{{\"pending_ulid\":\"{}\",\"event_id\":{}}}",
                ulid::to_crockford(pending_ulid),
                event_id
            )),
            rows_returned: None,
            rows_filtered: None,
            result: AuditResult::Success,
            reason: None,
            caller_ip: None,
            caller_ua: None,
            delegated_for_user_ulid: None,
        },
    )?;

    tx.commit()?;
    Ok((now, *pending_ulid))
}

/// Reject a pending event. Marks the row `rejected`, records an audit row
/// with the supplied reason. Returns the rejection timestamp.
pub fn reject_pending(
    conn: &mut Connection,
    pending_ulid: &Ulid,
    reason: Option<&str>,
) -> Result<i64> {
    let rand_tail = ulid::random_tail(pending_ulid);
    let tx = conn.transaction()?;

    let row: Option<(i64, i64, String)> = tx
        .query_row(
            "SELECT id, submitting_grant_id, status
               FROM pending_events WHERE ulid_random = ?1",
            params![rand_tail.to_vec()],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .ok();
    let Some((pending_id, grant_id, status)) = row else {
        return Err(Error::NotFound);
    };
    if status != "pending" {
        return Err(Error::InvalidArgument(format!(
            "pending row is in status {status:?}, not 'pending'"
        )));
    }

    let now = crate::format::now_ms();
    tx.execute(
        "UPDATE pending_events
            SET status = 'rejected',
                reviewed_at_ms = ?1,
                rejection_reason = ?2
          WHERE id = ?3",
        params![now, reason, pending_id],
    )?;

    audit::append(
        &tx,
        &AuditEntry {
            ts_ms: now,
            actor_type: audit::ActorType::Self_,
            auto_granted: false,
            grant_id: Some(grant_id),
            action: "pending_reject".into(),
            query_kind: Some("reject_pending".into()),
            query_params_json: Some(format!(
                "{{\"pending_ulid\":\"{}\"}}",
                ulid::to_crockford(pending_ulid)
            )),
            rows_returned: None,
            rows_filtered: None,
            result: AuditResult::Rejected,
            reason: reason.map(str::to_string),
            caller_ip: None,
            caller_ua: None,
            delegated_for_user_ulid: None,
        },
    )?;

    tx.commit()?;
    Ok(now)
}

/// Sweep `pending` rows whose `expires_at_ms` is in the past — flips them to
/// `expired`. Returns the number of rows touched.
pub fn sweep_expired(conn: &Connection, now_ms: i64) -> Result<u64> {
    let n = conn.execute(
        "UPDATE pending_events
            SET status = 'expired'
          WHERE status = 'pending' AND expires_at_ms <= ?1",
        params![now_ms],
    )?;
    Ok(n as u64)
}

fn parse_status(s: &str) -> Result<PendingStatus> {
    match s {
        "pending" => Ok(PendingStatus::Pending),
        "approved" => Ok(PendingStatus::Approved),
        "rejected" => Ok(PendingStatus::Rejected),
        "expired" => Ok(PendingStatus::Expired),
        other => Err(Error::InvalidArgument(format!(
            "unknown pending status {other:?}"
        ))),
    }
}

fn event_from_input(input: EventInput, ulid_bytes: &Ulid) -> Event {
    let channels: Vec<ChannelValue> = input.channels;
    Event {
        ulid: ulid::to_crockford(ulid_bytes),
        timestamp_ms: input.timestamp_ms,
        duration_ms: input.duration_ms,
        tz_offset_minutes: input.tz_offset_minutes,
        tz_name: input.tz_name,
        event_type: input.event_type,
        channels,
        sample_blocks: vec![],
        attachments: vec![],
        device_id: input.device_id,
        app_name: input.app_name,
        app_version: input.app_version,
        source: input.source,
        source_id: input.source_id,
        notes: input.notes,
        superseded_by: None,
        deleted_at_ms: None,
        top_level: input.top_level,
        signed_by: None,
    }
}

fn stitch_grant_ulid(rand_tail: &[u8]) -> Ulid {
    // Approximation: the grant's wire ULID encodes `(created_at_ms, rand_tail)`,
    // but the rule tables don't carry the time prefix here. v1 stitches a
    // zero-prefixed ULID so callers have a consistent display value; the
    // 80-bit random portion is the unique identity.
    let mut out = [0u8; 16];
    out[6..].copy_from_slice(rand_tail);
    out
}

fn fetch_event_ulid_by_rowid(conn: &Connection, event_id: i64) -> Result<Option<Ulid>> {
    let row: Option<(i64, Vec<u8>)> = conn
        .query_row(
            "SELECT timestamp_ms, ulid_random FROM events WHERE id = ?1",
            params![event_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    Ok(match row {
        Some((ts, rt)) => Some(ulid::from_parts(ts, &rt)?),
        None => None,
    })
}
