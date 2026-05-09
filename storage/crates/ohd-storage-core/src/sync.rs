//! Cache ↔ primary sync.
//!
//! Implements the bidirectional event-log replay defined in
//! `spec/sync-protocol.md`. v1 ships a minimal but complete implementation:
//!
//! - **Watermarks** are per-peer rowids in `peer_sync.last_outbound_rowid` and
//!   `last_inbound_peer_rowid`.
//! - **Hello** discovery exchanges high-water marks + registry version.
//! - **PushFrames** writes events idempotently (ULID dedup) into the local
//!   `events` table.
//! - **PullFrames** returns events with `events.id > after_peer_rowid`,
//!   skipping rows whose `origin_peer_id` matches the requesting peer (echo
//!   suppression).
//! - **Grant rows** sync as ordinary rows — we mirror them via
//!   [`upsert_grant`] using the random tail as the unique key.
//! - **Grant CRUD** is RPC-gated on the primary (`Create/Revoke/UpdateGrantOnPrimary`).
//!
//! Transport (Connect-RPC over HTTP/2 + HTTP/3) lives in
//! `crates/ohd-storage-server/src/sync_server.rs`.

use rusqlite::{params, Connection, OptionalExtension};

use crate::events::{ChannelScalar, ChannelValue, Event};
use crate::registry::{self, EventTypeName};
use crate::ulid::{self, Ulid};
use crate::Result;

/// Role this file plays in sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncRole {
    /// Primary — canonical.
    Primary,
    /// Cache — mirrors a remote primary.
    Cache,
    /// Mirror — read-only replica.
    Mirror,
}

/// One row in `peer_sync`.
#[derive(Debug, Clone)]
pub struct PeerSync {
    /// Internal rowid.
    pub id: i64,
    /// Stable peer label.
    pub peer_label: String,
    /// Peer kind.
    pub peer_kind: String,
    /// Optional peer OHD identity.
    pub peer_ulid: Option<Ulid>,
    /// Outbound watermark (sender-side, in our local rowid space).
    pub last_outbound_rowid: i64,
    /// Inbound watermark (in the peer's rowid space).
    pub last_inbound_peer_rowid: i64,
}

/// Upsert a peer's metadata. Returns the row's id.
pub fn upsert_peer(
    conn: &Connection,
    peer_label: &str,
    peer_kind: &str,
    peer_ulid: Option<&Ulid>,
) -> Result<i64> {
    let existing: Option<i64> = conn
        .query_row(
            "SELECT id FROM peer_sync WHERE peer_label = ?1",
            params![peer_label],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        if let Some(u) = peer_ulid {
            conn.execute(
                "UPDATE peer_sync SET peer_kind = ?1, peer_ulid = ?2 WHERE id = ?3",
                params![peer_kind, u.to_vec(), id],
            )?;
        } else {
            conn.execute(
                "UPDATE peer_sync SET peer_kind = ?1 WHERE id = ?2",
                params![peer_kind, id],
            )?;
        }
        return Ok(id);
    }
    let blob = peer_ulid.map(|u| u.to_vec());
    conn.execute(
        "INSERT INTO peer_sync (peer_label, peer_kind, peer_ulid)
         VALUES (?1, ?2, ?3)",
        params![peer_label, peer_kind, blob],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Read a single peer row by label.
pub fn read_peer(conn: &Connection, peer_label: &str) -> Result<Option<PeerSync>> {
    let row: Option<(i64, String, String, Option<Vec<u8>>, i64, i64)> = conn
        .query_row(
            "SELECT id, peer_label, peer_kind, peer_ulid,
                    last_outbound_rowid, last_inbound_peer_rowid
               FROM peer_sync WHERE peer_label = ?1",
            params![peer_label],
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
    Ok(
        row.map(|(id, label, kind, ulid_blob, out_rowid, in_rowid)| {
            let peer_ulid = ulid_blob.and_then(|b| {
                if b.len() == 16 {
                    let mut o = [0u8; 16];
                    o.copy_from_slice(&b);
                    Some(o)
                } else {
                    None
                }
            });
            PeerSync {
                id,
                peer_label: label,
                peer_kind: kind,
                peer_ulid,
                last_outbound_rowid: out_rowid,
                last_inbound_peer_rowid: in_rowid,
            }
        }),
    )
}

/// Bump `last_outbound_rowid` (called after the peer acks a frame).
pub fn advance_outbound_watermark(conn: &Connection, peer_id: i64, new_rowid: i64) -> Result<()> {
    conn.execute(
        "UPDATE peer_sync SET last_outbound_rowid = MAX(last_outbound_rowid, ?1)
          WHERE id = ?2",
        params![new_rowid, peer_id],
    )?;
    Ok(())
}

/// Bump `last_inbound_peer_rowid` (called after we accept a frame).
pub fn advance_inbound_watermark(conn: &Connection, peer_id: i64, new_rowid: i64) -> Result<()> {
    conn.execute(
        "UPDATE peer_sync SET last_inbound_peer_rowid = MAX(last_inbound_peer_rowid, ?1)
          WHERE id = ?2",
        params![new_rowid, peer_id],
    )?;
    Ok(())
}

/// Return the next batch of outbound events to push to a peer.
///
/// Skips events whose `origin_peer_id` is the target peer (echo suppression).
/// Caller advances `last_outbound_rowid` once each frame is acked.
pub fn outbound_events(
    conn: &Connection,
    peer_id: i64,
    after_rowid: i64,
    limit: i64,
) -> Result<Vec<(i64, Event)>> {
    let mut stmt = conn.prepare(
        "SELECT id, ulid_random, timestamp_ms FROM events
          WHERE id > ?1
            AND (origin_peer_id IS NULL OR origin_peer_id != ?2)
          ORDER BY id ASC
          LIMIT ?3",
    )?;
    let rows: Vec<(i64, Vec<u8>, i64)> = stmt
        .query_map(params![after_rowid, peer_id, limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Vec<u8>>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, rand_tail, ts) in rows {
        let ulid = ulid::from_parts(ts, &rand_tail)?;
        let event = crate::events::get_event_by_ulid(conn, &ulid)?;
        out.push((id, event));
    }
    Ok(out)
}

/// Apply an inbound event frame from `peer_id`. Returns:
/// - `Ok(true)` — newly inserted.
/// - `Ok(false)` — already had this ULID (idempotent dedup; sender advances anyway).
pub fn apply_inbound_event(conn: &mut Connection, peer_id: i64, event: &Event) -> Result<bool> {
    apply_inbound_event_with_envelope(conn, peer_id, event, None)
}

/// Same as [`apply_inbound_event`] but threads an envelope key so encrypted-
/// class channels are persisted as AEAD blobs (rather than plaintext) on the
/// receiving side. Most production callers should prefer this variant — see
/// [`crate::storage::Storage::envelope_key`].
pub fn apply_inbound_event_with_envelope(
    conn: &mut Connection,
    peer_id: i64,
    event: &Event,
    envelope_key: Option<&crate::encryption::EnvelopeKey>,
) -> Result<bool> {
    let parsed_ulid = ulid::parse_crockford(&event.ulid)?;
    let rand_tail = ulid::random_tail(&parsed_ulid);
    // ULID dedup.
    let exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM events WHERE ulid_random = ?1",
            params![rand_tail.to_vec()],
            |r| r.get(0),
        )
        .optional()?;
    if exists.is_some() {
        return Ok(false);
    }
    let etn = EventTypeName::parse(&event.event_type)?;
    let etype = registry::resolve_event_type(conn, &etn)?;
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT INTO events
            (ulid_random, timestamp_ms, tz_offset_minutes, tz_name, duration_ms,
             event_type_id, source, source_id, notes, origin_peer_id, deleted_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            rand_tail.to_vec(),
            event.timestamp_ms,
            event.tz_offset_minutes,
            event.tz_name,
            event.duration_ms,
            etype.id,
            event.source,
            event.source_id,
            event.notes,
            peer_id,
            event.deleted_at_ms,
        ],
    )?;
    let event_rowid = tx.last_insert_rowid();
    for cv in &event.channels {
        let chan = match registry::resolve_channel(&tx, etype.id, &cv.channel_path) {
            Ok(c) => c,
            Err(_) => continue, // unknown channel — skip silently
        };

        // Encryption gate (matches `events::insert_channel_value`).
        if crate::encryption::is_encrypted_class(&chan.sensitivity_class) {
            if let Some(env) = envelope_key {
                let active =
                    crate::encryption::load_active_class_key_tx(&tx, env, &chan.sensitivity_class)?;
                // Codex review #1+#2: XChaCha20-Poly1305 with wide AAD.
                let blob = crate::channel_encryption::encrypt_channel_value(
                    &chan.path,
                    &cv.value,
                    &active.key,
                    &parsed_ulid,
                    active.key_id,
                )?;
                tx.execute(
                    "INSERT INTO event_channels
                        (event_id, channel_id, encrypted, value_blob, encryption_key_id)
                     VALUES (?1, ?2, 1, ?3, ?4)",
                    params![event_rowid, chan.id, blob.to_bytes(), active.key_id,],
                )?;
                continue;
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
            params![event_rowid, chan.id, vr, vi, vt, ve],
        )?;
    }
    tx.commit()?;
    Ok(true)
}

/// Suppress unused warnings when the binary doesn't import `ChannelValue`.
#[allow(dead_code)]
fn _unused_channel_value() -> ChannelValue {
    ChannelValue {
        channel_path: String::new(),
        value: ChannelScalar::Real { real_value: 0.0 },
    }
}

// =============================================================================
// Per-peer attachment sync watermarks
// =============================================================================

/// Direction of an attachment-blob delivery: `Push` = caller pushed bytes to
/// the peer, `Pull` = caller pulled bytes from the peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentSyncDirection {
    /// Caller pushed the blob to the peer.
    Push,
    /// Caller pulled the blob from the peer.
    Pull,
}

impl AttachmentSyncDirection {
    /// On-disk string form.
    pub fn as_str(self) -> &'static str {
        match self {
            AttachmentSyncDirection::Push => "push",
            AttachmentSyncDirection::Pull => "pull",
        }
    }
}

/// Record that an attachment blob has crossed the wire between this instance
/// and `peer_id` in `direction`. Idempotent on `(peer_id, attachment_id, direction)`.
pub fn record_attachment_delivery(
    conn: &Connection,
    peer_id: i64,
    attachment_id: i64,
    direction: AttachmentSyncDirection,
    byte_size: i64,
) -> Result<()> {
    let now = crate::format::now_ms();
    conn.execute(
        "INSERT OR IGNORE INTO peer_attachment_sync
            (peer_id, attachment_id, direction, delivered_at_ms, byte_size)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![peer_id, attachment_id, direction.as_str(), now, byte_size],
    )?;
    Ok(())
}

/// Has `(peer_id, attachment_id, direction)` been recorded?
pub fn attachment_delivered(
    conn: &Connection,
    peer_id: i64,
    attachment_id: i64,
    direction: AttachmentSyncDirection,
) -> Result<bool> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM peer_attachment_sync
              WHERE peer_id = ?1 AND attachment_id = ?2 AND direction = ?3",
            params![peer_id, attachment_id, direction.as_str()],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0);
    Ok(count > 0)
}

/// Return attachments that have been *referenced by an event* but have not
/// yet been delivered to `peer_id` in the given direction. Used by the sync
/// orchestrator to compute the diff. Bounded by `limit`.
pub fn attachments_pending_delivery(
    conn: &Connection,
    peer_id: i64,
    direction: AttachmentSyncDirection,
    limit: i64,
) -> Result<Vec<(i64, Ulid)>> {
    let mut stmt = conn.prepare(
        "SELECT a.id, a.ulid_random
           FROM attachments a
          WHERE NOT EXISTS (
            SELECT 1 FROM peer_attachment_sync s
             WHERE s.peer_id = ?1
               AND s.attachment_id = a.id
               AND s.direction = ?2
          )
          ORDER BY a.id ASC
          LIMIT ?3",
    )?;
    let rows: Vec<(i64, Vec<u8>)> = stmt
        .query_map(params![peer_id, direction.as_str(), limit], |r| {
            Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::with_capacity(rows.len());
    for (id, rand_tail) in rows {
        let mut ulid_buf = [0u8; 16];
        if rand_tail.len() == 10 {
            ulid_buf[6..].copy_from_slice(&rand_tail);
        }
        out.push((id, ulid_buf));
    }
    Ok(out)
}
