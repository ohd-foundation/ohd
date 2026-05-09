//! Events, channel values, sample-block refs, attachments.
//!
//! Backs `events`, `event_channels`, `event_samples`, `attachments`. See
//! `spec/storage-format.md` "SQL schema" + "Validation on write" + "Sample blocks".

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::channel_encryption;
use crate::encryption::{self, EnvelopeKey};
use crate::registry::{self, ChannelRow, EventTypeName, EventTypeRow, ValueType};
use crate::ulid::{self, Ulid};
use crate::{Error, Result};

/// Sparse channel value attached to an event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelValue {
    /// Dot-separated channel path within the event's type.
    pub channel_path: String,
    /// Variant carries the typed scalar.
    #[serde(flatten)]
    pub value: ChannelScalar,
}

/// Typed channel scalar.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ChannelScalar {
    /// 64-bit float.
    Real {
        /// Real-typed scalar.
        real_value: f64,
    },
    /// 64-bit signed integer.
    Int {
        /// Int-typed scalar.
        int_value: i64,
    },
    /// Boolean.
    Bool {
        /// Bool-typed scalar.
        bool_value: bool,
    },
    /// Free text.
    Text {
        /// Text-typed scalar.
        text_value: String,
    },
    /// Append-only ordinal into the channel's enum_values.
    EnumOrdinal {
        /// Enum ordinal.
        enum_ordinal: i32,
    },
}

/// Reference to a compressed sample block (read side).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SampleBlockRef {
    /// Channel path the block lives on.
    pub channel_path: String,
    /// Absolute start (Unix ms, signed).
    pub t0_ms: i64,
    /// Absolute end (Unix ms, signed).
    pub t1_ms: i64,
    /// Number of samples in the block.
    pub sample_count: i32,
    /// Codec ID per `spec/storage-format.md` "Sample blocks".
    pub encoding: i32,
}

/// Reference to a sidecar attachment.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AttachmentRef {
    /// Wire ULID (Crockford-base32).
    pub ulid: String,
    /// 32-byte SHA-256 hex.
    pub sha256: String,
    /// Size in bytes.
    pub byte_size: i64,
    /// MIME type.
    pub mime_type: Option<String>,
    /// Original filename.
    pub filename: Option<String>,
}

/// One health event, wire shape.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Event {
    /// ULID in Crockford-base32 form.
    pub ulid: String,
    /// Signed Unix ms; negative = pre-1970.
    pub timestamp_ms: i64,
    /// Optional duration.
    pub duration_ms: Option<i64>,
    /// Local offset.
    pub tz_offset_minutes: Option<i32>,
    /// IANA zone name.
    pub tz_name: Option<String>,
    /// Namespaced event type, e.g. `"std.blood_glucose"`.
    pub event_type: String,
    /// Channel values.
    pub channels: Vec<ChannelValue>,
    /// Sample-block refs (payloads streamed via `ReadSamples`).
    pub sample_blocks: Vec<SampleBlockRef>,
    /// Attachment refs.
    pub attachments: Vec<AttachmentRef>,
    /// Logical device id.
    pub device_id: Option<String>,
    /// Recording app name.
    pub app_name: Option<String>,
    /// Recording app version.
    pub app_version: Option<String>,
    /// Source string.
    pub source: Option<String>,
    /// Idempotency key from upstream.
    pub source_id: Option<String>,
    /// Short freeform notes.
    pub notes: Option<String>,
    /// One-way pointer to a correction event.
    pub superseded_by: Option<String>,
    /// Soft-delete marker.
    pub deleted_at_ms: Option<i64>,
    /// Source-signing metadata when the event was committed with a verified
    /// `source_signature`. UI surfaces use this to render
    /// "signed by Libre" / "signed by Quest Diagnostics" badges.
    #[serde(default)]
    pub signed_by: Option<crate::source_signing::SignerInfo>,
}

/// Sparse representation used for writes.
///
/// The optional `source_signature` field carries an Ed25519 / RS256 / ES256
/// signature over the canonical-CBOR encoding of the event (see
/// [`crate::source_signing::canonical_event_bytes`]). When set, the
/// `signer_kid` is looked up in the `signers` registry and the signature
/// verified before the event is committed; verified events get a paired
/// `event_signatures` row that QueryEvents surfaces back via `signed_by`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EventInput {
    /// Measurement time.
    pub timestamp_ms: i64,
    /// Optional duration.
    pub duration_ms: Option<i64>,
    /// Local offset.
    pub tz_offset_minutes: Option<i32>,
    /// IANA zone name.
    pub tz_name: Option<String>,
    /// Namespaced event type.
    pub event_type: String,
    /// Channel values.
    pub channels: Vec<ChannelValue>,
    /// Logical device id.
    pub device_id: Option<String>,
    /// Recording app name.
    pub app_name: Option<String>,
    /// Recording app version.
    pub app_version: Option<String>,
    /// Source string.
    pub source: Option<String>,
    /// Idempotency key from upstream.
    pub source_id: Option<String>,
    /// Short freeform notes.
    pub notes: Option<String>,
    /// Sample-block payloads (pre-compressed by the codec). Optional. Each
    /// block writes one row to `event_samples` referencing the channel
    /// resolved against the event's type.
    #[serde(default)]
    pub sample_blocks: Vec<SampleBlockInput>,
    /// Optional source signature for high-trust integration writes. See the
    /// type doc on [`EventInput`] for the verification flow.
    #[serde(default)]
    pub source_signature: Option<crate::source_signing::SourceSignature>,
}

/// One pre-encoded sample block carried inline on `EventInput`. Mirrors the
/// proto's `SampleBlockInput`: client encodes via [`crate::sample_codec`] and
/// hands the compressed bytes to the server with codec metadata.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SampleBlockInput {
    /// Dotted channel path within the event's type.
    pub channel_path: String,
    /// Absolute start (Unix ms).
    pub t0_ms: i64,
    /// Absolute end (Unix ms).
    pub t1_ms: i64,
    /// Number of samples in the block.
    pub sample_count: i32,
    /// Codec ID (1 = float32, 2 = int16). See `spec/storage-format.md`.
    pub encoding: i32,
    /// Compressed payload bytes.
    pub data: Vec<u8>,
}

/// Filter for [`query_events`]. Mirrors most of the OHDC `EventFilter`; the
/// remaining bits (channel predicates with non-equality operators, full
/// channel-rule resolution) are noted in STATUS.md.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventFilter {
    /// Inclusive lower time bound.
    pub from_ms: Option<i64>,
    /// Inclusive upper time bound.
    pub to_ms: Option<i64>,
    /// Allowlist of dotted event-type names.
    #[serde(default)]
    pub event_types_in: Vec<String>,
    /// Denylist of dotted event-type names.
    #[serde(default)]
    pub event_types_not_in: Vec<String>,
    /// Whether to include soft-deleted events.
    #[serde(default)]
    pub include_deleted: bool,
    /// Whether to include superseded events (default true).
    #[serde(default = "default_true")]
    pub include_superseded: bool,
    /// Optional cap on the result size. v1 default 1000, max 10000.
    pub limit: Option<i64>,

    /// Restrict to events whose `events.device_id` resolves to one of these
    /// device-label strings (matches `devices.serial_or_id`).
    #[serde(default)]
    pub device_id_in: Vec<String>,
    /// Restrict to events whose `events.source` matches one of the supplied
    /// strings exactly. Spec calls out `source_in: ['health_connect:*']`-style
    /// glob patterns; v1 only does exact match — glob is a v1.x deliverable.
    #[serde(default)]
    pub source_in: Vec<String>,
    /// Restrict to a specific list of ULIDs (Crockford). Useful for case
    /// timeline materialization (a case carries an explicit event ULID list).
    #[serde(default)]
    pub event_ulids_in: Vec<String>,
    /// Sensitivity-class allowlist applied at the event-type level. Only
    /// events whose `event_type.default_sensitivity_class` is in this set are
    /// returned. Empty = no sensitivity filter.
    #[serde(default)]
    pub sensitivity_classes_in: Vec<String>,
    /// Sensitivity-class denylist applied at the event-type level.
    #[serde(default)]
    pub sensitivity_classes_not_in: Vec<String>,
    /// Channel-value predicates. AND-of-predicates only (no OR for v0).
    /// Evaluated as a post-query filter pass over the events that match the
    /// cheap predicates; perf trade-off documented in STATUS.md.
    #[serde(default)]
    pub channel_predicates: Vec<ChannelPredicate>,

    /// Case-scope expansion. Each ULID is resolved into the case's recursive
    /// scope (own filters + predecessor chain + child rollup) at query time
    /// via [`crate::cases::compute_case_scope`]. The expanded filters are
    /// OR-merged into the effective filter set.
    ///
    /// See `spec/storage-format.md` "Case scope resolution". Events are not
    /// tagged with `case_id`; the case's filters pull them in.
    #[serde(default)]
    pub case_ulids_in: Vec<String>,
}

fn default_true() -> bool {
    true
}

/// One channel-value predicate. `op` is one of `eq`, `neq`, `gt`, `gte`,
/// `lt`, `lte`. Reals support all six; ints/bool/text/enum support only
/// `eq`/`neq`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPredicate {
    /// Channel path within the event's type, e.g. `"value"` or `"mg_per_dl"`.
    pub channel_path: String,
    /// Operator: `"eq" | "neq" | "gt" | "gte" | "lt" | "lte"`.
    pub op: String,
    /// Comparand. The variant must match the channel's stored value type.
    pub value: ChannelScalar,
}

/// Comparison operator for [`ChannelPredicate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredicateOp {
    /// Equal.
    Eq,
    /// Not equal.
    Neq,
    /// Greater than.
    Gt,
    /// Greater than or equal.
    Gte,
    /// Less than.
    Lt,
    /// Less than or equal.
    Lte,
}

impl PredicateOp {
    /// Parse the wire string form.
    pub fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "eq" => PredicateOp::Eq,
            "neq" => PredicateOp::Neq,
            "gt" => PredicateOp::Gt,
            "gte" => PredicateOp::Gte,
            "lt" => PredicateOp::Lt,
            "lte" => PredicateOp::Lte,
            other => {
                return Err(Error::InvalidFilter(format!(
                    "unsupported predicate op {other:?}"
                )))
            }
        })
    }
}

/// Outcome of a single `put_events` row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PutEventResult {
    /// Event was committed to canonical storage.
    Committed {
        /// Crockford-base32 ULID.
        ulid: String,
        /// Commit timestamp.
        committed_at_ms: i64,
    },
    /// Event was queued in `pending_events` for user review.
    Pending {
        /// Crockford-base32 ULID (preserved across approval).
        ulid: String,
        /// Auto-expiry of the pending row.
        expires_at_ms: i64,
    },
    /// Event was rejected.
    Error {
        /// OHDC error code.
        code: String,
        /// Human-readable message.
        message: String,
    },
}

/// Internal helper used by the OHDC server: write a batch of events under the
/// caller's auth profile. Returns one outcome per input row.
///
/// `submitting_grant_id` is the grant rowid for grant/device tokens; `None`
/// for self-session callers (commit straight through with no approval queue).
///
/// `envelope_key` is the storage handle's live `K_envelope`. Required when
/// any input touches a channel whose `sensitivity_class` is in the
/// encrypted-classes set; pass `None` only on the testing-only no-cipher-key
/// path. When `None` and an encrypted-class channel is encountered, that
/// channel is written in plaintext (with a warning trace) — the migration is
/// safe but the resulting row isn't operator-private.
pub fn put_events(
    conn: &mut Connection,
    inputs: &[EventInput],
    submitting_grant_id: Option<i64>,
    require_approval: bool,
    envelope_key: Option<&EnvelopeKey>,
) -> Result<Vec<PutEventResult>> {
    let tx = conn.transaction()?;
    let mut out = Vec::with_capacity(inputs.len());
    for input in inputs {
        let res = match write_one(
            &tx,
            input,
            submitting_grant_id,
            require_approval,
            envelope_key,
        ) {
            Ok(r) => r,
            Err(Error::IdempotencyConflict) => PutEventResult::Error {
                code: "IDEMPOTENCY_CONFLICT".into(),
                message: "duplicate (source, source_id) with different content".into(),
            },
            Err(e) => PutEventResult::Error {
                code: e.code().to_string(),
                message: e.to_string(),
            },
        };
        out.push(res);
    }
    tx.commit()?;
    Ok(out)
}

fn write_one(
    tx: &Transaction<'_>,
    input: &EventInput,
    submitting_grant_id: Option<i64>,
    require_approval: bool,
    envelope_key: Option<&EnvelopeKey>,
) -> Result<PutEventResult> {
    let event_type_name = EventTypeName::parse(&input.event_type)?;
    let etype = registry::resolve_event_type(tx, &event_type_name)?;

    // Resolve all channel definitions up front; reject early on unknown.
    let mut resolved: Vec<(ChannelRow, &ChannelValue)> = Vec::new();
    for cv in &input.channels {
        let chan = registry::resolve_channel(tx, etype.id, &cv.channel_path)?;
        validate_channel_value(&chan, cv)?;
        resolved.push((chan, cv));
    }

    // Idempotency check — see `idx_events_dedup`.
    if let (Some(src), Some(sid)) = (&input.source, &input.source_id) {
        let dup: Option<(i64, Vec<u8>, i64)> = tx
            .query_row(
                "SELECT id, ulid_random, timestamp_ms FROM events
                  WHERE source = ?1 AND source_id = ?2",
                params![src, sid],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((_id, rand_tail, ts)) = dup {
            // Exact replay → return the existing ULID; differing payload is conflict.
            // For v1 we return the existing ULID; full content-equality check is v1.x.
            let ulid = ulid::from_parts(ts, &rand_tail)?;
            return Ok(PutEventResult::Committed {
                ulid: ulid::to_crockford(&ulid),
                committed_at_ms: ts,
            });
        }
    }

    // Allocate the ULID up front (preserved across pending → committed).
    let new_ulid = ulid::mint(input.timestamp_ms);
    let rand_tail = ulid::random_tail(&new_ulid);

    // Source signing: verify BEFORE any DB mutation so the rejection is
    // clean (no orphan rows). When the signature is present and verifies
    // OK, we'll record the signature row after the event row is inserted.
    if let Some(sig) = input.source_signature.as_ref() {
        crate::source_signing::verify_signature(tx, input, &new_ulid, sig)?;
    }

    if require_approval {
        // Route through pending_events.
        let grant_id = submitting_grant_id.ok_or(Error::InvalidArgument(
            "require_approval without grant token".into(),
        ))?;
        let now = crate::format::now_ms();
        let expires = now + 7 * 86_400_000; // 7-day default
        let payload = serde_json::to_string(input)?;
        tx.execute(
            "INSERT INTO pending_events
                (ulid_random, submitted_at_ms, submitting_grant_id, payload_json,
                 status, expires_at_ms)
             VALUES (?1, ?2, ?3, ?4, 'pending', ?5)",
            params![rand_tail.to_vec(), now, grant_id, payload, expires],
        )?;
        return Ok(PutEventResult::Pending {
            ulid: ulid::to_crockford(&new_ulid),
            expires_at_ms: expires,
        });
    }

    // Resolve / upsert provenance helpers (device, app_version).
    let device_id = match &input.device_id {
        Some(label) => Some(upsert_device(tx, label)?),
        None => None,
    };
    let app_id = match (&input.app_name, &input.app_version) {
        (Some(name), Some(ver)) => Some(upsert_app_version(tx, name, ver)?),
        _ => None,
    };

    // Insert into events.
    tx.execute(
        "INSERT INTO events
            (ulid_random, timestamp_ms, tz_offset_minutes, tz_name, duration_ms,
             event_type_id, device_id, app_id, source, source_id, notes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            rand_tail.to_vec(),
            input.timestamp_ms,
            input.tz_offset_minutes,
            input.tz_name,
            input.duration_ms,
            etype.id,
            device_id,
            app_id,
            input.source,
            input.source_id,
            input.notes,
        ],
    )?;
    let event_rowid = tx.last_insert_rowid();

    // Insert channel values. Channels whose `sensitivity_class` is in the
    // encrypted-classes set go through the AEAD pipeline; everything else
    // takes the existing plaintext path.
    for (chan, cv) in &resolved {
        insert_channel_value(tx, event_rowid, &new_ulid, chan, cv, envelope_key)?;
    }

    // Record the source signature row (already verified above).
    if let Some(sig) = input.source_signature.as_ref() {
        crate::source_signing::record_signature(tx, event_rowid, sig)?;
    }

    // Insert sample blocks (P1 / P0 wiring).
    //
    // Each block resolves its channel via the registry under the event's type;
    // unknown channels error out at the boundary (we don't accept partial
    // success because sample blocks are part of the event's atomic write).
    for (block_index, block) in input.sample_blocks.iter().enumerate() {
        let chan = registry::resolve_channel(tx, etype.id, &block.channel_path)?;
        crate::samples::insert_sample_block(
            tx,
            event_rowid,
            chan.id,
            block_index as i32,
            block.t0_ms,
            block.t1_ms,
            block.sample_count,
            block.encoding,
            &block.data,
        )?;
    }

    Ok(PutEventResult::Committed {
        ulid: ulid::to_crockford(&new_ulid),
        committed_at_ms: input.timestamp_ms,
    })
}

fn upsert_device(tx: &Transaction<'_>, label: &str) -> Result<i64> {
    // Treat label as a synthetic kind+serial pair for v1 (full device row
    // shape is OHDC's DeviceRow message; the smoke test passes a free-form id).
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM devices WHERE serial_or_id = ?1",
            params![label],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    tx.execute(
        "INSERT INTO devices (kind, serial_or_id) VALUES ('manual', ?1)",
        params![label],
    )?;
    Ok(tx.last_insert_rowid())
}

fn upsert_app_version(tx: &Transaction<'_>, name: &str, version: &str) -> Result<i64> {
    let existing: Option<i64> = tx
        .query_row(
            "SELECT id FROM app_versions WHERE app_name = ?1 AND version = ?2 AND platform IS NULL",
            params![name, version],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    tx.execute(
        "INSERT INTO app_versions (app_name, version) VALUES (?1, ?2)",
        params![name, version],
    )?;
    Ok(tx.last_insert_rowid())
}

fn insert_channel_value(
    tx: &Transaction<'_>,
    event_id: i64,
    event_ulid: &Ulid,
    chan: &ChannelRow,
    cv: &ChannelValue,
    envelope_key: Option<&EnvelopeKey>,
) -> Result<()> {
    // Channels in the encrypted-classes set go through the AEAD pipeline.
    // We need the envelope key to unwrap the per-class DEK; without one we
    // fall back to plaintext (testing-only no-cipher-key path) and emit a
    // tracing warning so production misconfiguration is loud.
    if encryption::is_encrypted_class(&chan.sensitivity_class) {
        match envelope_key {
            Some(env) => {
                let active =
                    encryption::load_active_class_key_tx(tx, env, &chan.sensitivity_class)?;
                // Codex review #1+#2: XChaCha20-Poly1305 with AAD bound to
                // (channel_path, event_ulid, key_id).
                let blob = channel_encryption::encrypt_channel_value(
                    &chan.path,
                    &cv.value,
                    &active.key,
                    event_ulid,
                    active.key_id,
                )?;
                tx.execute(
                    "INSERT INTO event_channels
                        (event_id, channel_id, encrypted, value_blob, encryption_key_id)
                     VALUES (?1, ?2, 1, ?3, ?4)",
                    params![event_id, chan.id, blob.to_bytes(), active.key_id],
                )?;
                return Ok(());
            }
            None => {
                tracing::warn!(
                    channel_path = %chan.path,
                    sensitivity_class = %chan.sensitivity_class,
                    "writing encrypted-class channel as plaintext: no envelope key available \
                     (testing-only no-cipher-key configuration)"
                );
                // fall through to the plaintext path
            }
        }
    }

    let (vr, vi, vt, ve): (Option<f64>, Option<i64>, Option<String>, Option<i32>) = match &cv.value
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
    Ok(())
}

fn validate_channel_value(chan: &ChannelRow, cv: &ChannelValue) -> Result<()> {
    let bad_value_type = || Error::WrongValueType(cv.channel_path.clone());
    match (chan.value_type, &cv.value) {
        (ValueType::Real, ChannelScalar::Real { .. }) => Ok(()),
        (ValueType::Int, ChannelScalar::Int { .. }) => Ok(()),
        (ValueType::Bool, ChannelScalar::Bool { .. }) => Ok(()),
        (ValueType::Text, ChannelScalar::Text { .. }) => Ok(()),
        (ValueType::Enum, ChannelScalar::EnumOrdinal { enum_ordinal }) => {
            if (*enum_ordinal as usize) < chan.enum_values.len() {
                Ok(())
            } else {
                Err(Error::InvalidEnum(cv.channel_path.clone()))
            }
        }
        (ValueType::Group, _) => Err(bad_value_type()),
        _ => Err(bad_value_type()),
    }
}

/// Fetch a single event by its (decoded) ULID.
///
/// Encrypted channel values are decrypted in-place using `envelope_key` (the
/// storage handle's live `K_envelope`). When `envelope_key` is `None`,
/// encrypted channels surface as redacted markers via
/// [`channel_encryption::redacted_marker`] — that path matches what a
/// grant-token holder without the wrap material sees and is also what the
/// testing-only no-cipher-key configuration produces.
pub fn get_event_by_ulid_with_key(
    conn: &Connection,
    ulid: &Ulid,
    envelope_key: Option<&EnvelopeKey>,
) -> Result<Event> {
    get_event_by_ulid_inner(conn, ulid, envelope_key)
}

/// Backwards-compatible wrapper that uses no envelope key. Encrypted channels
/// surface as redacted markers (see [`channel_encryption::redacted_marker`]).
/// Most callers should prefer [`get_event_by_ulid_with_key`].
pub fn get_event_by_ulid(conn: &Connection, ulid: &Ulid) -> Result<Event> {
    get_event_by_ulid_inner(conn, ulid, None)
}

fn get_event_by_ulid_inner(
    conn: &Connection,
    ulid: &Ulid,
    envelope_key: Option<&EnvelopeKey>,
) -> Result<Event> {
    let rand_tail = ulid::random_tail(ulid);
    let row: Option<(
        i64,
        i64,
        Option<i32>,
        Option<String>,
        Option<i64>,
        i64,
        Option<i64>,
        Option<i64>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<i64>,
        Option<i64>,
    )> = conn
        .query_row(
            "SELECT id, timestamp_ms, tz_offset_minutes, tz_name, duration_ms,
                    event_type_id, device_id, app_id, source, source_id, notes,
                    superseded_by, deleted_at_ms
               FROM events WHERE ulid_random = ?1",
            params![rand_tail.to_vec()],
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
                    r.get(10)?,
                    r.get(11)?,
                    r.get(12)?,
                ))
            },
        )
        .optional()?;
    let (
        event_id,
        ts,
        tz_off,
        tz_name,
        dur,
        et_id,
        device_id,
        app_id,
        source,
        source_id,
        notes,
        superseded_by,
        deleted_at,
    ) = row.ok_or(Error::NotFound)?;
    let etype = registry::event_type_by_id(conn, et_id)?;
    let channels = load_channels(conn, event_id, ulid, envelope_key)?;
    let device_label = match device_id {
        Some(id) => conn
            .query_row(
                "SELECT serial_or_id FROM devices WHERE id = ?1",
                params![id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten(),
        None => None,
    };
    let (app_name, app_version) = match app_id {
        Some(id) => conn
            .query_row(
                "SELECT app_name, version FROM app_versions WHERE id = ?1",
                params![id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
            )
            .map(|(n, v)| (Some(n), Some(v)))
            .unwrap_or((None, None)),
        None => (None, None),
    };
    let superseded_by_str = match superseded_by {
        Some(rid) => conn
            .query_row(
                "SELECT timestamp_ms, ulid_random FROM events WHERE id = ?1",
                params![rid],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Vec<u8>>(1)?)),
            )
            .ok()
            .and_then(|(ts2, rt2)| ulid::from_parts(ts2, &rt2).ok())
            .map(|u| ulid::to_crockford(&u)),
        None => None,
    };

    let sample_blocks = crate::samples::list_blocks(conn, event_id)?;
    let attachments = crate::attachments::list_for_event(conn, event_id)?;
    let signed_by = crate::source_signing::signer_info_for_event(conn, event_id)?;

    Ok(Event {
        ulid: ulid::to_crockford(ulid),
        timestamp_ms: ts,
        duration_ms: dur,
        tz_offset_minutes: tz_off,
        tz_name,
        event_type: format!("{}.{}", etype.namespace, etype.name),
        channels,
        sample_blocks,
        attachments,
        device_id: device_label,
        app_name,
        app_version,
        source,
        source_id,
        notes,
        superseded_by: superseded_by_str,
        deleted_at_ms: deleted_at,
        signed_by,
    })
}

fn load_channels(
    conn: &Connection,
    event_id: i64,
    event_ulid: &Ulid,
    envelope_key: Option<&EnvelopeKey>,
) -> Result<Vec<ChannelValue>> {
    let mut stmt = conn.prepare(
        "SELECT c.path, c.value_type, c.enum_values, c.sensitivity_class,
                ec.value_real, ec.value_int, ec.value_text, ec.value_enum,
                ec.encrypted, ec.value_blob, ec.encryption_key_id
           FROM event_channels ec
           JOIN channels c ON c.id = ec.channel_id
          WHERE ec.event_id = ?1",
    )?;
    type Row = (
        String,          // path
        String,          // value_type
        Option<String>,  // enum_values
        String,          // sensitivity_class
        Option<f64>,     // value_real
        Option<i64>,     // value_int
        Option<String>,  // value_text
        Option<i32>,     // value_enum
        i64,             // encrypted (0/1)
        Option<Vec<u8>>, // value_blob
        Option<i64>,     // encryption_key_id
    );
    let raw_rows: Vec<Row> = stmt
        .query_map(params![event_id], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
                r.get::<_, i64>(8)?,
                r.get(9)?,
                r.get(10)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    let mut out = Vec::with_capacity(raw_rows.len());
    for (path, vt, _enum_values, sensitivity_class, vr, vi, vt_text, ve, encrypted, blob, key_id) in
        raw_rows
    {
        let scalar = if encrypted != 0 {
            decode_encrypted_channel(
                conn,
                &path,
                &sensitivity_class,
                blob.as_deref(),
                key_id,
                event_ulid,
                envelope_key,
            )?
        } else {
            match vt.as_str() {
                "real" => ChannelScalar::Real {
                    real_value: vr.unwrap_or(0.0),
                },
                "int" => ChannelScalar::Int {
                    int_value: vi.unwrap_or(0),
                },
                "bool" => ChannelScalar::Bool {
                    bool_value: vi.unwrap_or(0) != 0,
                },
                "text" => ChannelScalar::Text {
                    text_value: vt_text.unwrap_or_default(),
                },
                "enum" => ChannelScalar::EnumOrdinal {
                    enum_ordinal: ve.unwrap_or(0),
                },
                _ => ChannelScalar::Text {
                    text_value: String::new(),
                },
            }
        };
        out.push(ChannelValue {
            channel_path: path,
            value: scalar,
        });
    }
    Ok(out)
}

/// Decode an `encrypted=1` row.
///
/// Returns the redacted marker (`<encrypted: $sensitivity_class>`) when:
/// - The supplied `envelope_key` is `None` (caller doesn't hold the wrap
///   material — e.g. a grant token without the right `class_key_wraps`).
/// - The blob is malformed (returns the marker rather than failing the entire
///   query — the row exists, the grantee just sees "this is private").
///
/// Errors when the on-disk row is structurally invalid (missing blob /
/// key_id) — that's an internal state corruption, not a privacy decision.
fn decode_encrypted_channel(
    conn: &Connection,
    channel_path: &str,
    sensitivity_class: &str,
    blob_bytes: Option<&[u8]>,
    key_id: Option<i64>,
    event_ulid: &Ulid,
    envelope_key: Option<&EnvelopeKey>,
) -> Result<ChannelScalar> {
    let blob_bytes = match blob_bytes {
        Some(b) => b,
        None => {
            tracing::error!(
                channel_path,
                "encrypted=1 row without value_blob — corrupt; returning redacted marker"
            );
            return Ok(channel_encryption::redacted_marker(sensitivity_class));
        }
    };
    let key_id = match key_id {
        Some(k) => k,
        None => {
            tracing::error!(
                channel_path,
                "encrypted=1 row without encryption_key_id — corrupt; returning redacted marker"
            );
            return Ok(channel_encryption::redacted_marker(sensitivity_class));
        }
    };
    let env = match envelope_key {
        Some(e) => e,
        None => return Ok(channel_encryption::redacted_marker(sensitivity_class)),
    };
    let class_key = match encryption::load_class_key_by_id(conn, env, sensitivity_class, key_id) {
        Ok(k) => k,
        Err(e) => {
            tracing::warn!(
                channel_path,
                key_id,
                "failed to unwrap class key: {e}; returning redacted marker"
            );
            return Ok(channel_encryption::redacted_marker(sensitivity_class));
        }
    };
    let blob = match channel_encryption::EncryptedBlob::from_bytes(blob_bytes) {
        Ok(b) => b,
        Err(_) => return Ok(channel_encryption::redacted_marker(sensitivity_class)),
    };
    match channel_encryption::decrypt_channel_value(
        channel_path,
        &blob,
        &class_key,
        event_ulid,
        key_id,
    ) {
        Ok(s) => Ok(s),
        Err(_) => Ok(channel_encryption::redacted_marker(sensitivity_class)),
    }
}

/// List events matching a filter. v1 supports `from_ms` / `to_ms`,
/// event-type allow/deny lists, `device_id_in`, `source_in`, `event_ulids_in`,
/// sensitivity-class allow/deny (at event-type granularity), and AND-of
/// channel predicates (post-query filter pass over the cheap-predicate match
/// set).
///
/// Returns `(events, rows_filtered)` where `rows_filtered` is the number of
/// matching rows the grant scope dropped silently. Self-session callers always
/// see `rows_filtered=0`.
///
/// Backwards-compatible variant that does not decrypt encrypted-class
/// channels (they surface as redacted markers). Most callers should prefer
/// [`query_events_with_key`].
pub fn query_events(
    conn: &Connection,
    filter: &EventFilter,
    grant_scope: Option<&GrantScope>,
) -> Result<(Vec<Event>, i64)> {
    query_events_inner(conn, filter, grant_scope, None)
}

/// Same as [`query_events`] but threads an envelope key for value-level
/// decryption. The envelope key comes from
/// [`crate::storage::Storage::envelope_key`].
pub fn query_events_with_key(
    conn: &Connection,
    filter: &EventFilter,
    grant_scope: Option<&GrantScope>,
    envelope_key: Option<&EnvelopeKey>,
) -> Result<(Vec<Event>, i64)> {
    query_events_inner(conn, filter, grant_scope, envelope_key)
}

fn query_events_inner(
    conn: &Connection,
    filter: &EventFilter,
    grant_scope: Option<&GrantScope>,
    envelope_key: Option<&EnvelopeKey>,
) -> Result<(Vec<Event>, i64)> {
    // Case-scope expansion: when `case_ulids_in` is non-empty, expand each
    // ULID into the case's recursive scope (own filters + predecessor chain
    // + child rollup) and OR-merge by running one query per filter and
    // de-duplicating by ULID. The outer filter's other constraints (limit,
    // sort, channel predicates) apply on the merged set.
    if !filter.case_ulids_in.is_empty() {
        return query_events_with_case_scope(conn, filter, grant_scope, envelope_key);
    }
    let mut sql = String::from(
        "SELECT id, ulid_random, timestamp_ms, event_type_id
           FROM events WHERE 1=1",
    );
    let mut args: Vec<rusqlite::types::Value> = Vec::new();
    if !filter.include_deleted {
        sql.push_str(" AND deleted_at_ms IS NULL");
    }
    if !filter.include_superseded {
        sql.push_str(" AND superseded_by IS NULL");
    }
    if let Some(from) = filter.from_ms {
        sql.push_str(" AND timestamp_ms >= ?");
        args.push(from.into());
    }
    if let Some(to) = filter.to_ms {
        sql.push_str(" AND timestamp_ms <= ?");
        args.push(to.into());
    }
    if !filter.event_types_in.is_empty() {
        let mut ids = Vec::new();
        for t in &filter.event_types_in {
            let n = EventTypeName::parse(t)?;
            if let Ok(et) = registry::resolve_event_type(conn, &n) {
                ids.push(et.id);
            }
        }
        if ids.is_empty() {
            return Ok((vec![], 0));
        }
        let placeholders = (0..ids.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        sql.push_str(&format!(" AND event_type_id IN ({placeholders})"));
        for id in ids {
            args.push(id.into());
        }
    }
    if !filter.event_types_not_in.is_empty() {
        let mut ids = Vec::new();
        for t in &filter.event_types_not_in {
            let n = EventTypeName::parse(t)?;
            if let Ok(et) = registry::resolve_event_type(conn, &n) {
                ids.push(et.id);
            }
        }
        if !ids.is_empty() {
            let placeholders = (0..ids.len()).map(|_| "?").collect::<Vec<_>>().join(",");
            sql.push_str(&format!(" AND event_type_id NOT IN ({placeholders})"));
            for id in ids {
                args.push(id.into());
            }
        }
    }
    if !filter.device_id_in.is_empty() {
        // Resolve device labels into rowids; missing labels are silently
        // ignored (a query asking about a device that's never been seen
        // returns zero rows, not an error).
        let mut device_ids: Vec<i64> = Vec::new();
        for label in &filter.device_id_in {
            if let Ok(id) = conn.query_row(
                "SELECT id FROM devices WHERE serial_or_id = ?1",
                rusqlite::params![label],
                |r| r.get::<_, i64>(0),
            ) {
                device_ids.push(id);
            }
        }
        if device_ids.is_empty() {
            return Ok((vec![], 0));
        }
        let placeholders = (0..device_ids.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(" AND device_id IN ({placeholders})"));
        for id in device_ids {
            args.push(id.into());
        }
    }
    if !filter.source_in.is_empty() {
        let placeholders = (0..filter.source_in.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(" AND source IN ({placeholders})"));
        for s in &filter.source_in {
            args.push(s.clone().into());
        }
    }
    if !filter.event_ulids_in.is_empty() {
        // Decode the Crockford strings into 10-byte rand-tails; that's what
        // `events.ulid_random` indexes on.
        let mut tails: Vec<Vec<u8>> = Vec::new();
        for s in &filter.event_ulids_in {
            let u = ulid::parse_crockford(s)?;
            tails.push(ulid::random_tail(&u).to_vec());
        }
        if tails.is_empty() {
            return Ok((vec![], 0));
        }
        let placeholders = (0..tails.len()).map(|_| "?").collect::<Vec<_>>().join(",");
        sql.push_str(&format!(" AND ulid_random IN ({placeholders})"));
        for t in tails {
            args.push(t.into());
        }
    }
    if !filter.sensitivity_classes_in.is_empty() {
        let placeholders = (0..filter.sensitivity_classes_in.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(
            " AND event_type_id IN (SELECT id FROM event_types WHERE default_sensitivity_class IN ({placeholders}))"
        ));
        for s in &filter.sensitivity_classes_in {
            args.push(s.clone().into());
        }
    }
    if !filter.sensitivity_classes_not_in.is_empty() {
        let placeholders = (0..filter.sensitivity_classes_not_in.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        sql.push_str(&format!(
            " AND event_type_id NOT IN (SELECT id FROM event_types WHERE default_sensitivity_class IN ({placeholders}))"
        ));
        for s in &filter.sensitivity_classes_not_in {
            args.push(s.clone().into());
        }
    }
    sql.push_str(" ORDER BY timestamp_ms DESC");
    let limit = filter.limit.unwrap_or(1000).min(10_000);
    sql.push_str(&format!(" LIMIT {limit}"));

    let mut stmt = conn.prepare(&sql)?;
    let rows: Vec<(i64, Vec<u8>, i64, i64)> = stmt
        .query_map(rusqlite::params_from_iter(args.iter()), |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;

    // Pre-parse channel predicates once.
    let mut parsed_predicates: Vec<(String, PredicateOp, ChannelScalar)> =
        Vec::with_capacity(filter.channel_predicates.len());
    for p in &filter.channel_predicates {
        parsed_predicates.push((
            p.channel_path.clone(),
            PredicateOp::parse(&p.op)?,
            p.value.clone(),
        ));
    }

    let mut out = Vec::new();
    let mut filtered = 0i64;
    for (_id, rand_tail, ts, et_id) in rows {
        // Apply grant scope intersection — full precedence ladder.
        if let Some(scope) = grant_scope {
            // Time-window rule (rolling + absolute).
            if !scope.allows_timestamp(ts) {
                filtered += 1;
                continue;
            }
            // Sensitivity-deny + event-type-deny + event-type-allow + default.
            let etype = registry::event_type_by_id(conn, et_id)?;
            if !scope.allows_event_sensitivity(&etype.default_sensitivity_class) {
                filtered += 1;
                continue;
            }
            if !scope.allows_event_type(et_id) {
                filtered += 1;
                continue;
            }
        }
        let ulid = ulid::from_parts(ts, &rand_tail)?;
        let mut e = get_event_by_ulid_inner(conn, &ulid, envelope_key)?;
        // Channel-level filtering (sensitivity + channel rules) happens here
        // because the channel's sensitivity_class only resolves once we've
        // loaded the channel rows for the event.
        if let Some(scope) = grant_scope {
            apply_channel_scope(conn, et_id, &mut e, scope)?;
        }
        // Channel predicates evaluated post-query.
        if !parsed_predicates.is_empty() && !channel_predicates_match(&e, &parsed_predicates) {
            // These are filter predicates (caller's intent), not grant scope —
            // count them as "didn't match", not as "rows_filtered" (which is
            // reserved for silent grant-scope drops).
            continue;
        }
        // Apply strip_notes / aggregation_only on the grant.
        if let Some(scope) = grant_scope {
            if scope.strip_notes {
                e.notes = None;
            }
        }
        out.push(e);
    }
    Ok((out, filtered))
}

/// Apply per-channel grant-scope filtering to an already-loaded event. Looks
/// up each channel's `sensitivity_class` and `id` against the scope's
/// allow/deny rules; channels that don't pass are stripped from the event's
/// `channels` vec (per-channel deny is silent — the grantee never sees
/// stripped channels).
fn apply_channel_scope(
    conn: &Connection,
    et_id: i64,
    e: &mut Event,
    scope: &GrantScope,
) -> Result<()> {
    let mut keep: Vec<ChannelValue> = Vec::with_capacity(e.channels.len());
    for cv in std::mem::take(&mut e.channels) {
        let chan = match registry::resolve_channel(conn, et_id, &cv.channel_path) {
            Ok(c) => c,
            // Unknown channel slipped through — keep it (defensive; the
            // schema's UNIQUE constraint plus put_events validation should
            // prevent this case ever surfacing).
            Err(_) => {
                keep.push(cv);
                continue;
            }
        };
        if scope.allows_channel(chan.id, &chan.sensitivity_class) {
            keep.push(cv);
        }
    }
    e.channels = keep;
    Ok(())
}

/// Evaluate the AND-of channel predicates against an event's channel values.
fn channel_predicates_match(
    event: &Event,
    predicates: &[(String, PredicateOp, ChannelScalar)],
) -> bool {
    for (path, op, comparand) in predicates {
        let Some(cv) = event.channels.iter().find(|c| &c.channel_path == path) else {
            return false;
        };
        if !scalar_satisfies(&cv.value, *op, comparand) {
            return false;
        }
    }
    true
}

fn scalar_satisfies(actual: &ChannelScalar, op: PredicateOp, comparand: &ChannelScalar) -> bool {
    use ChannelScalar::*;
    use PredicateOp::*;
    match (actual, comparand) {
        (Real { real_value: a }, Real { real_value: b }) => match op {
            Eq => a == b,
            Neq => a != b,
            Gt => a > b,
            Gte => a >= b,
            Lt => a < b,
            Lte => a <= b,
        },
        (Int { int_value: a }, Int { int_value: b }) => match op {
            Eq => a == b,
            Neq => a != b,
            Gt => a > b,
            Gte => a >= b,
            Lt => a < b,
            Lte => a <= b,
        },
        (Bool { bool_value: a }, Bool { bool_value: b }) => match op {
            Eq => a == b,
            Neq => a != b,
            _ => false,
        },
        (Text { text_value: a }, Text { text_value: b }) => match op {
            Eq => a == b,
            Neq => a != b,
            _ => false,
        },
        (EnumOrdinal { enum_ordinal: a }, EnumOrdinal { enum_ordinal: b }) => match op {
            Eq => a == b,
            Neq => a != b,
            _ => false,
        },
        // Mismatched scalars never satisfy. Caller is responsible for
        // ensuring the predicate's value type matches the channel's.
        _ => false,
    }
}

/// Materialized grant rules for a single read query.
///
/// Implements the precedence ladder from `spec/storage-format.md`
/// "Combination precedence (resolution edge cases)":
///
/// 1. **Sensitivity-class deny** (any deny matching the event's or channel's
///    `sensitivity_class`)
/// 2. **Channel deny** (any channel-id deny)
/// 3. **Event-type deny**
/// 4. **Sensitivity-class allow**
/// 5. **Channel allow**
/// 6. **Event-type allow**
/// 7. `default_action` (the fallback)
#[derive(Debug, Clone, Default)]
pub struct GrantScope {
    /// Default action for events not covered by any explicit rule.
    pub default_allow: bool,
    /// Event-type-id allowlist (rules with effect='allow').
    pub event_type_allow: Vec<i64>,
    /// Event-type-id denylist (rules with effect='deny').
    pub event_type_deny: Vec<i64>,
    /// Sensitivity-class allowlist.
    pub sensitivity_allow: Vec<String>,
    /// Sensitivity-class denylist (full string match against
    /// `event_types.default_sensitivity_class` and the channel's own
    /// `sensitivity_class`).
    pub sensitivity_deny: Vec<String>,
    /// Channel-id allowlist (per `grant_channel_rules`).
    pub channel_allow: Vec<i64>,
    /// Channel-id denylist (per `grant_channel_rules`).
    pub channel_deny: Vec<i64>,
    /// `events.notes` is replaced with `None` on returned rows.
    pub strip_notes: bool,
    /// Rolling-window time bound: events older than `now - days*86400_000`
    /// are denied.
    pub rolling_window_days: Option<i32>,
    /// Absolute time window: events outside `[from_ms, to_ms]` are denied.
    pub absolute_window: Option<(i64, i64)>,
    /// Whether this grant has a per-day rate limit; if exceeded, return
    /// `RateLimited`. The actual count is checked at the OHDC layer using
    /// `audit_log` rows for this grant.
    pub max_queries_per_day: Option<i32>,
    /// Per-hour rate limit (same enforcement model as per_day).
    pub max_queries_per_hour: Option<i32>,
    /// Materialized "now" used to evaluate rolling windows. Captured once at
    /// `grant_scope_for` time so a slow query doesn't see a sliding cutoff.
    pub now_ms: i64,
}

impl GrantScope {
    /// Cheap event-type-id check using the precedence ladder.
    /// (Channel-level filtering happens after channels are loaded.)
    pub fn allows_event_type(&self, et_id: i64) -> bool {
        // 3. Event-type deny.
        if self.event_type_deny.contains(&et_id) {
            return false;
        }
        // 6. Event-type allow.
        if self.event_type_allow.contains(&et_id) {
            return true;
        }
        // 7. Default action.
        self.default_allow
    }

    /// Check the event's timestamp against time-window rules. Returns true
    /// when the event is within the window (or no window applies).
    pub fn allows_timestamp(&self, ts_ms: i64) -> bool {
        if let Some(days) = self.rolling_window_days {
            let cutoff = self.now_ms.saturating_sub(days as i64 * 86_400_000);
            if ts_ms < cutoff {
                return false;
            }
        }
        if let Some((from, to)) = self.absolute_window {
            if ts_ms < from || ts_ms > to {
                return false;
            }
        }
        true
    }

    /// Apply sensitivity-class deny against an event-type's default class.
    pub fn allows_event_sensitivity(&self, sensitivity_class: &str) -> bool {
        // 1. Sensitivity deny.
        !self.sensitivity_deny.iter().any(|s| s == sensitivity_class)
    }

    /// Decide whether to keep a single channel (path + sensitivity_class +
    /// channel_id) for an allowed event. Implements steps 1, 2, 4, 5, plus
    /// (default-action fall-through).
    pub fn allows_channel(&self, channel_id: i64, sensitivity_class: &str) -> bool {
        // 1. Sensitivity-class deny.
        if self.sensitivity_deny.iter().any(|s| s == sensitivity_class) {
            return false;
        }
        // 2. Channel deny.
        if self.channel_deny.contains(&channel_id) {
            return false;
        }
        // 4. Sensitivity-class allow.
        if self
            .sensitivity_allow
            .iter()
            .any(|s| s == sensitivity_class)
        {
            return true;
        }
        // 5. Channel allow.
        if self.channel_allow.contains(&channel_id) {
            return true;
        }
        // 7. Default action — channels inherit from the grant default. (Step 6
        // is event-type allow, which already applied at the row level.)
        self.default_allow
    }
}

/// Used by [`get_event_by_ulid_scoped`].
pub fn get_event_by_ulid_scoped(
    conn: &Connection,
    ulid: &Ulid,
    scope: Option<&GrantScope>,
) -> Result<Event> {
    get_event_by_ulid_scoped_with_key(conn, ulid, scope, None)
}

/// Same as [`get_event_by_ulid_scoped`] but threads an envelope key for
/// value-level decryption of encrypted-class channels.
pub fn get_event_by_ulid_scoped_with_key(
    conn: &Connection,
    ulid: &Ulid,
    scope: Option<&GrantScope>,
    envelope_key: Option<&EnvelopeKey>,
) -> Result<Event> {
    let event = get_event_by_ulid_inner(conn, ulid, envelope_key)?;
    if let Some(scope) = scope {
        // Re-resolve event_type_id for the scope check.
        let etn = EventTypeName::parse(&event.event_type)?;
        let et = registry::resolve_event_type(conn, &etn)?;
        if !scope.allows_event_type(et.id) {
            return Err(Error::NotFound);
        }
    }
    Ok(event)
}

/// Convenience: resolve a wire event-type name (for testing).
#[allow(dead_code)]
pub(crate) fn _resolve(conn: &Connection, name: &str) -> Result<EventTypeRow> {
    registry::resolve_event_type(conn, &EventTypeName::parse(name)?)
}

/// Helper: when [`EventFilter::case_ulids_in`] is set, expand each case ULID
/// into its recursive case scope, run one query per expanded filter, and
/// de-duplicate by event ULID. The outer filter's `from_ms`/`to_ms`,
/// event-type allow/deny, channel predicates, and limit/sort all apply on
/// the merged set as a final pass.
fn query_events_with_case_scope(
    conn: &Connection,
    outer: &EventFilter,
    grant_scope: Option<&GrantScope>,
    envelope_key: Option<&EnvelopeKey>,
) -> Result<(Vec<Event>, i64)> {
    use crate::cases;
    use std::collections::BTreeMap;

    // Expand each case ULID into its scope filters.
    let mut expanded: Vec<EventFilter> = Vec::new();
    for crockford in &outer.case_ulids_in {
        let case_ulid = ulid::parse_crockford(crockford)?;
        let case_id = cases::case_id_by_ulid(conn, &case_ulid)?;
        let scope = cases::compute_case_scope(conn, case_id)?;
        if scope.is_empty() {
            // A case with no filters (yet) contributes nothing. Skip silently
            // — the case row exists, just has no membership rules.
            continue;
        }
        expanded.extend(scope);
    }
    if expanded.is_empty() {
        // No filters in the case-scope set: return empty. The caller asked
        // for case-scoped events; without any case filters there's nothing
        // to return.
        return Ok((vec![], 0));
    }

    // Run each expanded filter (intersected with the outer filter's
    // constraints other than `case_ulids_in`) and merge.
    let mut merged: BTreeMap<String, Event> = BTreeMap::new();
    let mut total_filtered: i64 = 0;
    for f in &expanded {
        let intersected = intersect_filters(outer, f);
        let (events, dropped) = query_events_inner(conn, &intersected, grant_scope, envelope_key)?;
        total_filtered += dropped;
        for e in events {
            merged.entry(e.ulid.clone()).or_insert(e);
        }
    }

    let mut out: Vec<Event> = merged.into_values().collect();
    // Re-sort by timestamp_ms DESC (matches the non-case-scope path's order).
    out.sort_by(|a, b| b.timestamp_ms.cmp(&a.timestamp_ms));

    // Apply the outer limit if specified (default 1000).
    let limit = outer.limit.unwrap_or(1000).min(10_000) as usize;
    if out.len() > limit {
        out.truncate(limit);
    }

    Ok((out, total_filtered))
}

/// Build a filter that's the intersection of the outer (caller-supplied)
/// filter and one inner case-scope filter. The case scope contributes its
/// own constraints (e.g. device_id_in, time range from `case_filters`); the
/// outer contributes user intent. Without `case_ulids_in` to avoid recursion.
fn intersect_filters(outer: &EventFilter, inner: &EventFilter) -> EventFilter {
    EventFilter {
        from_ms: max_opt(outer.from_ms, inner.from_ms),
        to_ms: min_opt(outer.to_ms, inner.to_ms),
        // Event-type allow/deny: union the two allow lists when both empty
        // (case scope has none) or merge intersection when both non-empty.
        // The simple-but-correct rule: if either side restricts, AND them.
        event_types_in: intersect_lists(&outer.event_types_in, &inner.event_types_in),
        event_types_not_in: union_lists(&outer.event_types_not_in, &inner.event_types_not_in),
        include_deleted: outer.include_deleted && inner.include_deleted,
        include_superseded: outer.include_superseded && inner.include_superseded,
        limit: outer.limit, // outer limits the merged set; inner queries return raw matches
        device_id_in: intersect_lists(&outer.device_id_in, &inner.device_id_in),
        source_in: intersect_lists(&outer.source_in, &inner.source_in),
        event_ulids_in: if outer.event_ulids_in.is_empty() {
            inner.event_ulids_in.clone()
        } else if inner.event_ulids_in.is_empty() {
            outer.event_ulids_in.clone()
        } else {
            // Both side have explicit ULID lists — intersection.
            intersect_lists(&outer.event_ulids_in, &inner.event_ulids_in)
        },
        sensitivity_classes_in: intersect_lists(
            &outer.sensitivity_classes_in,
            &inner.sensitivity_classes_in,
        ),
        sensitivity_classes_not_in: union_lists(
            &outer.sensitivity_classes_not_in,
            &inner.sensitivity_classes_not_in,
        ),
        channel_predicates: {
            let mut all = outer.channel_predicates.clone();
            all.extend(inner.channel_predicates.iter().cloned());
            all
        },
        // Crucial: clear case_ulids_in to prevent infinite recursion when the
        // intersected filter is dispatched through `query_events`.
        case_ulids_in: vec![],
    }
}

fn max_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

fn min_opt(a: Option<i64>, b: Option<i64>) -> Option<i64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) | (None, Some(x)) => Some(x),
        (None, None) => None,
    }
}

/// Intersection of two string lists. If either side is empty (= "no
/// restriction"), returns the other side.
fn intersect_lists(a: &[String], b: &[String]) -> Vec<String> {
    if a.is_empty() {
        return b.to_vec();
    }
    if b.is_empty() {
        return a.to_vec();
    }
    a.iter().filter(|s| b.contains(s)).cloned().collect()
}

/// Union of two string lists, deduplicated.
fn union_lists(a: &[String], b: &[String]) -> Vec<String> {
    let mut out = a.to_vec();
    for s in b {
        if !out.contains(s) {
            out.push(s.clone());
        }
    }
    out
}
