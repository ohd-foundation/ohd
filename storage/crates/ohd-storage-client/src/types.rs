//! Plain owned request / response structs for [`OhdcRemoteClient`].
//!
//! These mirror the field set of the uniffi DTOs in `ohd-storage-bindings`
//! (`EventDto`, `GrantDto`, …) but carry **no uniffi derives** — this crate
//! has no uniffi dependency. `ohd-storage-bindings` maps these 1:1 onto its
//! DTOs so the Android `StorageRepository` sees identical shapes whether the
//! backend is local or remote.
//!
//! ULIDs are raw 16-byte vectors. The uniffi layer renders them as
//! Crockford-base32 (it already links `ohd-storage-core`'s `ulid` module).

/// Discriminant for [`ChannelValue`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    /// `f64`.
    Real,
    /// `i64`.
    Int,
    /// `bool`.
    Bool,
    /// `String`.
    Text,
    /// Append-only enum ordinal.
    EnumOrdinal,
}

/// A typed channel scalar. Exactly one `*_value` field is set per `value_kind`.
#[derive(Debug, Clone)]
pub struct ChannelValue {
    /// Channel path within the event's type (e.g. `"value"`).
    pub channel_path: String,
    /// Which value variant is set.
    pub value_kind: ValueKind,
    /// Real-typed scalar.
    pub real_value: Option<f64>,
    /// Int-typed scalar.
    pub int_value: Option<i64>,
    /// Bool-typed scalar.
    pub bool_value: Option<bool>,
    /// Text-typed scalar.
    pub text_value: Option<String>,
    /// Enum ordinal.
    pub enum_ordinal: Option<i32>,
}

/// Sparse event input for [`OhdcRemoteClient::put_event`].
#[derive(Debug, Clone)]
pub struct EventInput {
    /// Measurement time (signed Unix ms).
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
    /// Idempotency key.
    pub source_id: Option<String>,
    /// Notes.
    pub notes: Option<String>,
}

/// One stored event.
#[derive(Debug, Clone)]
pub struct Event {
    /// Raw 16-byte ULID.
    pub ulid: Vec<u8>,
    /// Signed Unix ms.
    pub timestamp_ms: i64,
    /// Duration.
    pub duration_ms: Option<i64>,
    /// Event type.
    pub event_type: String,
    /// Channels.
    pub channels: Vec<ChannelValue>,
    /// Optional notes.
    pub notes: Option<String>,
    /// Source.
    pub source: Option<String>,
    /// Soft-delete marker.
    pub deleted_at_ms: Option<i64>,
}

/// Outcome of a single `put_event` call.
#[derive(Debug, Clone)]
pub struct PutEventOutcome {
    /// `"committed" | "pending" | "error"`.
    pub outcome: String,
    /// Raw 16-byte ULID for committed/pending; empty for errors.
    pub ulid: Vec<u8>,
    /// Wall-clock ms when committed; pending expiry for pending; 0 for errors.
    pub timestamp_ms: i64,
    /// OHDC error code; empty unless `outcome == "error"`.
    pub error_code: String,
    /// Human-readable message; empty unless `outcome == "error"`.
    pub error_message: String,
}

/// Filter for [`OhdcRemoteClient::query_events`] / `count_events`.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    /// Inclusive lower time bound.
    pub from_ms: Option<i64>,
    /// Inclusive upper time bound.
    pub to_ms: Option<i64>,
    /// Allowlist of dotted event-type names.
    pub event_types_in: Vec<String>,
    /// Denylist of dotted event-type names.
    pub event_types_not_in: Vec<String>,
    /// Whether to include soft-deleted events.
    pub include_deleted: bool,
    /// Result cap.
    pub limit: Option<i64>,
    /// Restrict to events with `source` exactly in this list.
    pub source_in: Vec<String>,
}

/// One row of [`OhdcRemoteClient::list_event_types`] — a distinct
/// `event_type` name + its count within the supplied filter.
#[derive(Debug, Clone)]
pub struct EventTypeSummary {
    pub event_type: String,
    pub count: i64,
}

/// Filter for [`OhdcRemoteClient::delete_events`]. All fields optional; an
/// empty filter wipes every event owned by the authenticated identity.
#[derive(Debug, Clone, Default)]
pub struct DeleteFilter {
    /// Inclusive lower bound on `timestamp_ms`. `None` = no lower bound.
    pub from_ms: Option<i64>,
    /// Inclusive upper bound. `None` = no upper bound.
    pub to_ms: Option<i64>,
    /// Restrict to these event-type names. Empty = all types.
    pub event_types: Vec<String>,
}

/// Filter for [`OhdcRemoteClient::list_grants`].
#[derive(Debug, Clone, Default)]
pub struct ListGrantsFilter {
    /// Include revoked grants.
    pub include_revoked: bool,
    /// Include hard-expired grants.
    pub include_expired: bool,
    /// Filter by grantee_kind exact match.
    pub grantee_kind: Option<String>,
    /// Page size.
    pub limit: Option<i64>,
}

/// Per-event-type rule (allow or deny).
#[derive(Debug, Clone)]
pub struct GrantEventTypeRule {
    /// Dotted event-type name.
    pub event_type: String,
    /// `"allow"` or `"deny"`.
    pub effect: String,
}

/// Per-channel rule.
#[derive(Debug, Clone)]
pub struct GrantChannelRule {
    /// Dotted event-type name.
    pub event_type: String,
    /// Channel path within that type.
    pub channel_path: String,
    /// `"allow"` or `"deny"`.
    pub effect: String,
}

/// Per-sensitivity-class rule.
#[derive(Debug, Clone)]
pub struct GrantSensitivityRule {
    /// `"general" | "mental_health" | …`.
    pub sensitivity_class: String,
    /// `"allow"` or `"deny"`.
    pub effect: String,
}

/// Materialized grant row.
#[derive(Debug, Clone)]
pub struct Grant {
    /// Raw 16-byte ULID.
    pub ulid: Vec<u8>,
    /// Grantee display label.
    pub grantee_label: String,
    /// Grantee kind.
    pub grantee_kind: String,
    /// Free-text purpose.
    pub purpose: Option<String>,
    /// Creation timestamp (Unix ms).
    pub created_at_ms: i64,
    /// Hard-expiry; `None` = no hard expiry.
    pub expires_at_ms: Option<i64>,
    /// Revocation timestamp; `None` = active.
    pub revoked_at_ms: Option<i64>,
    /// `"allow"` or `"deny"`.
    pub default_action: String,
    /// `"always" | "auto_for_event_types" | "never_required"`.
    pub approval_mode: String,
    /// Aggregation-only.
    pub aggregation_only: bool,
    /// Strip notes on returned rows.
    pub strip_notes: bool,
    /// Notify on each access.
    pub notify_on_access: bool,
    /// Per-event-type read rules.
    pub event_type_rules: Vec<GrantEventTypeRule>,
    /// Per-channel read rules.
    pub channel_rules: Vec<GrantChannelRule>,
    /// Per-sensitivity-class read rules.
    pub sensitivity_rules: Vec<GrantSensitivityRule>,
    /// Auto-approve event-type allowlist.
    pub auto_approve_event_types: Vec<String>,
}

/// Sparse builder for [`OhdcRemoteClient::create_grant`].
#[derive(Debug, Clone)]
pub struct CreateGrantInput {
    /// Display label for the grantee.
    pub grantee_label: String,
    /// Grantee kind.
    pub grantee_kind: String,
    /// Free-text purpose.
    pub purpose: Option<String>,
    /// `"allow"` or `"deny"`.
    pub default_action: String,
    /// `"always" | "auto_for_event_types" | "never_required"`.
    pub approval_mode: String,
    /// Hard-expiry timestamp (Unix ms).
    pub expires_at_ms: Option<i64>,
    /// Per-event-type read rules.
    pub event_type_rules: Vec<GrantEventTypeRule>,
    /// Per-channel read rules.
    pub channel_rules: Vec<GrantChannelRule>,
    /// Per-sensitivity-class read rules.
    pub sensitivity_rules: Vec<GrantSensitivityRule>,
    /// Per-event-type write rules.
    pub write_event_type_rules: Vec<GrantEventTypeRule>,
    /// Auto-approve event-type allowlist.
    pub auto_approve_event_types: Vec<String>,
    /// Aggregation-only flag.
    pub aggregation_only: bool,
    /// Strip notes on returned rows.
    pub strip_notes: bool,
    /// Notify on every access.
    pub notify_on_access: bool,
}

/// Result of [`OhdcRemoteClient::create_grant`].
#[derive(Debug, Clone)]
pub struct GrantToken {
    /// New grant ULID (raw 16 bytes).
    pub grant_ulid: Vec<u8>,
    /// Cleartext bearer token (`ohdg_…`).
    pub token: String,
    /// Convenience share URL.
    pub share_url: String,
}

/// One pending event row.
#[derive(Debug, Clone)]
pub struct PendingEvent {
    /// Pending ULID (raw 16 bytes).
    pub ulid: Vec<u8>,
    /// Submission time (Unix ms).
    pub submitted_at_ms: i64,
    /// Submitting grant ULID, when resolvable (raw 16 bytes).
    pub submitting_grant_ulid: Option<Vec<u8>>,
    /// `"pending" | "approved" | "rejected" | "expired"`.
    pub status: String,
    /// Review time.
    pub reviewed_at_ms: Option<i64>,
    /// Optional rejection reason.
    pub rejection_reason: Option<String>,
    /// Auto-expiry (Unix ms).
    pub expires_at_ms: i64,
    /// The materialized event.
    pub event: Event,
}

/// One case row.
#[derive(Debug, Clone)]
pub struct Case {
    /// Raw 16-byte ULID.
    pub ulid: Vec<u8>,
    /// Type tag.
    pub case_type: String,
    /// Optional human-readable label.
    pub case_label: Option<String>,
    /// Start time (Unix ms).
    pub started_at_ms: i64,
    /// Close time (`None` = ongoing).
    pub ended_at_ms: Option<i64>,
    /// Parent case (raw 16 bytes).
    pub parent_case_ulid: Option<Vec<u8>>,
    /// Predecessor case (raw 16 bytes).
    pub predecessor_case_ulid: Option<Vec<u8>>,
    /// Authority that opened the case (raw 16 bytes).
    pub opening_authority_grant_ulid: Option<Vec<u8>>,
    /// Inactivity threshold (hours).
    pub inactivity_close_after_h: Option<i32>,
    /// Last activity timestamp.
    pub last_activity_at_ms: i64,
}

/// Filter for [`OhdcRemoteClient::audit_query`].
#[derive(Debug, Clone, Default)]
pub struct AuditFilter {
    /// Inclusive lower time bound (Unix ms).
    pub from_ms: Option<i64>,
    /// Inclusive upper time bound (Unix ms).
    pub to_ms: Option<i64>,
    /// Filter by actor type string.
    pub actor_type: Option<String>,
    /// Filter by action string.
    pub action: Option<String>,
    /// Filter by result string.
    pub result: Option<String>,
    /// Client-side cap on collected rows.
    pub limit: Option<i64>,
}

/// One audit log entry.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    /// Time of the operation.
    pub ts_ms: i64,
    /// Actor type.
    pub actor_type: String,
    /// Action label.
    pub action: String,
    /// Sub-classification.
    pub query_kind: Option<String>,
    /// JSON-encoded request payload.
    pub query_params_json: Option<String>,
    /// Returned rows.
    pub rows_returned: Option<i64>,
    /// Filtered rows.
    pub rows_filtered: Option<i64>,
    /// Result string.
    pub result: String,
    /// Failure reason.
    pub reason: Option<String>,
}

/// Result of [`OhdcRemoteClient::whoami`].
#[derive(Debug, Clone)]
pub struct WhoAmI {
    /// Calling user's ULID (raw 16 bytes).
    pub user_ulid: Vec<u8>,
    /// `"self_session" | "grant" | "device"`.
    pub token_kind: String,
    /// Grant ULID when the token is a grant bearer (raw 16 bytes).
    pub grant_ulid: Option<Vec<u8>>,
    /// Grantee label when present.
    pub grantee_label: Option<String>,
    /// Server-observed caller IP.
    pub caller_ip: String,
}

/// Result of [`OhdcRemoteClient::health`].
#[derive(Debug, Clone)]
pub struct Health {
    /// `"ok" | "degraded" | "down"`.
    pub status: String,
    /// Server wall clock (Unix ms).
    pub server_time_ms: i64,
    /// Server build version.
    pub server_version: String,
    /// OHDC protocol version (e.g. `"ohdc.v0"`).
    pub protocol_version: String,
}
