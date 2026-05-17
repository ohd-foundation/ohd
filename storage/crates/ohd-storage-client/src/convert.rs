//! Conversions between the wire `pb` types and the crate's plain owned
//! structs ([`crate::types`]).
//!
//! The request side (plain → pb) mirrors the inverse of the server's
//! `*_pb_to_core` helpers; the response side (pb → plain) mirrors the
//! inverse of the server's `*_core_to_pb` helpers. Keeping the mapping here
//! — rather than in the uniffi wrapper — means `ohd-storage-bindings` never
//! has to name a `buffa` type.

use crate::pb;
use crate::types::*;

// =============================================================================
// ULID helpers
// =============================================================================

/// Wrap raw 16-byte ULID into a wire `pb::Ulid`.
pub(crate) fn ulid_to_pb(bytes: Vec<u8>) -> pb::Ulid {
    pb::Ulid {
        bytes,
        ..Default::default()
    }
}

/// Extract the raw 16-byte ULID from a wire `MessageField<Ulid>`.
fn ulid_from_field(field: ::buffa::MessageField<pb::Ulid>) -> Vec<u8> {
    field.into_option().map(|u| u.bytes).unwrap_or_default()
}

/// Extract an optional raw ULID from a `MessageField<Ulid>` (absent → `None`).
fn opt_ulid_from_field(field: ::buffa::MessageField<pb::Ulid>) -> Option<Vec<u8>> {
    field
        .into_option()
        .map(|u| u.bytes)
        .filter(|b| !b.is_empty())
}

// =============================================================================
// ChannelValue
// =============================================================================

/// Plain → wire.
fn channel_value_to_pb(cv: ChannelValue) -> pb::ChannelValue {
    use pb::channel_value::Value;
    let value = match cv.value_kind {
        ValueKind::Real => cv.real_value.map(Value::RealValue),
        ValueKind::Int => cv.int_value.map(Value::IntValue),
        ValueKind::Bool => cv.bool_value.map(Value::BoolValue),
        ValueKind::Text => cv.text_value.map(Value::TextValue),
        ValueKind::EnumOrdinal => cv.enum_ordinal.map(Value::EnumOrdinal),
    };
    pb::ChannelValue {
        channel_path: cv.channel_path,
        value,
        ..Default::default()
    }
}

/// Wire → plain.
fn channel_value_from_pb(cv: pb::ChannelValue) -> ChannelValue {
    use pb::channel_value::Value;
    let mut out = ChannelValue {
        channel_path: cv.channel_path,
        value_kind: ValueKind::Real,
        real_value: None,
        int_value: None,
        bool_value: None,
        text_value: None,
        enum_ordinal: None,
    };
    match cv.value {
        Some(Value::RealValue(v)) => {
            out.value_kind = ValueKind::Real;
            out.real_value = Some(v);
        }
        Some(Value::IntValue(v)) => {
            out.value_kind = ValueKind::Int;
            out.int_value = Some(v);
        }
        Some(Value::BoolValue(v)) => {
            out.value_kind = ValueKind::Bool;
            out.bool_value = Some(v);
        }
        Some(Value::TextValue(v)) => {
            out.value_kind = ValueKind::Text;
            out.text_value = Some(v);
        }
        Some(Value::EnumOrdinal(v)) => {
            out.value_kind = ValueKind::EnumOrdinal;
            out.enum_ordinal = Some(v);
        }
        None => {}
    }
    out
}

// =============================================================================
// Events
// =============================================================================

/// Plain → wire.
pub(crate) fn event_input_to_pb(input: EventInput) -> pb::EventInput {
    pb::EventInput {
        timestamp_ms: input.timestamp_ms,
        duration_ms: input.duration_ms,
        tz_offset_minutes: input.tz_offset_minutes,
        tz_name: input.tz_name,
        event_type: input.event_type,
        channels: input.channels.into_iter().map(channel_value_to_pb).collect(),
        device_id: input.device_id,
        app_name: input.app_name,
        app_version: input.app_version,
        source: input.source,
        source_id: input.source_id,
        notes: input.notes,
        ..Default::default()
    }
}

/// Wire → plain.
pub(crate) fn event_from_pb(e: pb::Event) -> Event {
    Event {
        ulid: ulid_from_field(e.ulid),
        timestamp_ms: e.timestamp_ms,
        duration_ms: e.duration_ms,
        event_type: e.event_type,
        channels: e.channels.into_iter().map(channel_value_from_pb).collect(),
        notes: e.notes,
        source: e.source,
        deleted_at_ms: e.deleted_at_ms,
    }
}

/// Plain → wire.
pub(crate) fn event_filter_to_pb(f: EventFilter) -> pb::EventFilter {
    pb::EventFilter {
        from_ms: f.from_ms,
        to_ms: f.to_ms,
        event_types_in: f.event_types_in,
        event_types_not_in: f.event_types_not_in,
        source_in: f.source_in,
        include_deleted: f.include_deleted,
        // The local `query_events` always sets include_superseded = true.
        include_superseded: true,
        limit: f.limit,
        ..Default::default()
    }
}

/// Wire `PutEventResult` → plain outcome.
pub(crate) fn put_event_result_from_pb(r: pb::PutEventResult) -> PutEventOutcome {
    use pb::put_event_result::Outcome;
    match r.outcome {
        Some(Outcome::Committed(c)) => PutEventOutcome {
            outcome: "committed".to_string(),
            ulid: ulid_from_field(c.ulid),
            timestamp_ms: c.committed_at_ms,
            error_code: String::new(),
            error_message: String::new(),
        },
        Some(Outcome::Pending(p)) => PutEventOutcome {
            outcome: "pending".to_string(),
            ulid: ulid_from_field(p.ulid),
            timestamp_ms: p.expires_at_ms,
            error_code: String::new(),
            error_message: String::new(),
        },
        Some(Outcome::Error(e)) => PutEventOutcome {
            outcome: "error".to_string(),
            ulid: Vec::new(),
            timestamp_ms: 0,
            error_code: e.code,
            error_message: e.message,
        },
        None => PutEventOutcome {
            outcome: "error".to_string(),
            ulid: Vec::new(),
            timestamp_ms: 0,
            error_code: "INTERNAL".to_string(),
            error_message: "PutEventResult had no outcome".to_string(),
        },
    }
}

// =============================================================================
// Grants
// =============================================================================

fn grant_rule_effect(s: &str) -> String {
    match s {
        "allow" => "allow".to_string(),
        _ => "deny".to_string(),
    }
}

/// Wire → plain.
pub(crate) fn grant_from_pb(g: pb::Grant) -> Grant {
    Grant {
        ulid: ulid_from_field(g.ulid),
        grantee_label: g.grantee_label,
        grantee_kind: g.grantee_kind,
        purpose: g.purpose,
        created_at_ms: g.created_at_ms,
        expires_at_ms: g.expires_at_ms,
        revoked_at_ms: g.revoked_at_ms,
        default_action: g.default_action,
        approval_mode: g.approval_mode,
        aggregation_only: g.aggregation_only,
        strip_notes: g.strip_notes,
        notify_on_access: g.notify_on_access,
        event_type_rules: g
            .event_type_rules
            .into_iter()
            .map(|r| GrantEventTypeRule {
                event_type: r.event_type,
                effect: grant_rule_effect(&r.effect),
            })
            .collect(),
        channel_rules: g
            .channel_rules
            .into_iter()
            .map(|r| {
                // The wire `GrantChannelRule.channel_path` is the full dotted
                // path; recover `(event_type, channel_path)` the same way the
                // server's `split_grant_channel_path` does.
                let (event_type, channel_path) = split_grant_channel_path(&r.channel_path);
                GrantChannelRule {
                    event_type,
                    channel_path,
                    effect: grant_rule_effect(&r.effect),
                }
            })
            .collect(),
        sensitivity_rules: g
            .sensitivity_rules
            .into_iter()
            .map(|r| GrantSensitivityRule {
                sensitivity_class: r.sensitivity_class,
                effect: grant_rule_effect(&r.effect),
            })
            .collect(),
        auto_approve_event_types: g.auto_approve_event_types,
    }
}

/// Split a full dotted grant `channel_path` into `(event_type, channel_path)`.
/// Mirrors the server's heuristic.
fn split_grant_channel_path(path: &str) -> (String, String) {
    let mut parts = path.splitn(4, '.');
    match (parts.next(), parts.next()) {
        (Some("com"), Some(p1)) => {
            let p2 = parts.next().unwrap_or_default();
            let rest = parts.next().unwrap_or_default();
            (format!("com.{p1}.{p2}"), rest.to_string())
        }
        (Some(p0), Some(p1)) => {
            let rest: String = parts.collect::<Vec<_>>().join(".");
            (format!("{p0}.{p1}"), rest)
        }
        _ => (path.to_string(), String::new()),
    }
}

/// Plain → wire `CreateGrantRequest`.
pub(crate) fn create_grant_to_pb(input: CreateGrantInput) -> pb::CreateGrantRequest {
    pb::CreateGrantRequest {
        grantee_label: input.grantee_label,
        grantee_kind: input.grantee_kind,
        purpose: input.purpose,
        default_action: input.default_action,
        approval_mode: input.approval_mode,
        expires_at_ms: input.expires_at_ms,
        aggregation_only: input.aggregation_only,
        strip_notes: input.strip_notes,
        notify_on_access: input.notify_on_access,
        event_type_rules: input
            .event_type_rules
            .into_iter()
            .map(|r| pb::GrantEventTypeRule {
                event_type: r.event_type,
                effect: r.effect,
                ..Default::default()
            })
            .collect(),
        channel_rules: input
            .channel_rules
            .into_iter()
            .map(|r| pb::GrantChannelRule {
                // The wire rule carries the full dotted path.
                channel_path: format!("{}.{}", r.event_type, r.channel_path),
                effect: r.effect,
                ..Default::default()
            })
            .collect(),
        sensitivity_rules: input
            .sensitivity_rules
            .into_iter()
            .map(|r| pb::GrantSensitivityRule {
                sensitivity_class: r.sensitivity_class,
                effect: r.effect,
                ..Default::default()
            })
            .collect(),
        write_event_type_rules: input
            .write_event_type_rules
            .into_iter()
            .map(|r| pb::GrantWriteEventTypeRule {
                event_type: r.event_type,
                effect: r.effect,
                ..Default::default()
            })
            .collect(),
        auto_approve_event_types: input.auto_approve_event_types,
        ..Default::default()
    }
}

// =============================================================================
// Pending events
// =============================================================================

/// Wire → plain.
pub(crate) fn pending_from_pb(p: pb::PendingEvent) -> PendingEvent {
    PendingEvent {
        ulid: ulid_from_field(p.ulid),
        submitted_at_ms: p.submitted_at_ms,
        submitting_grant_ulid: opt_ulid_from_field(p.submitting_grant_ulid),
        status: p.status,
        reviewed_at_ms: p.reviewed_at_ms,
        rejection_reason: p.rejection_reason,
        expires_at_ms: p.expires_at_ms,
        event: p
            .event
            .into_option()
            .map(event_from_pb)
            .unwrap_or(Event {
                ulid: Vec::new(),
                timestamp_ms: 0,
                duration_ms: None,
                event_type: String::new(),
                channels: Vec::new(),
                notes: None,
                source: None,
                deleted_at_ms: None,
            }),
    }
}

// =============================================================================
// Cases
// =============================================================================

/// Wire → plain.
pub(crate) fn case_from_pb(c: pb::Case) -> Case {
    Case {
        ulid: ulid_from_field(c.ulid),
        case_type: c.case_type,
        case_label: c.case_label,
        started_at_ms: c.started_at_ms,
        ended_at_ms: c.ended_at_ms,
        parent_case_ulid: opt_ulid_from_field(c.parent_case_ulid),
        predecessor_case_ulid: opt_ulid_from_field(c.predecessor_case_ulid),
        opening_authority_grant_ulid: opt_ulid_from_field(c.opening_authority_grant_ulid),
        inactivity_close_after_h: c.inactivity_close_after_h,
        last_activity_at_ms: c.last_activity_at_ms,
    }
}

// =============================================================================
// Audit
// =============================================================================

/// Wire → plain. Empty strings become `None` for the optional-shaped fields.
pub(crate) fn audit_entry_from_pb(e: pb::AuditEntry) -> AuditEntry {
    AuditEntry {
        ts_ms: e.ts_ms,
        actor_type: e.actor_type,
        action: e.action,
        query_kind: Some(e.query_kind).filter(|s| !s.is_empty()),
        query_params_json: Some(e.query_params_json).filter(|s| !s.is_empty()),
        rows_returned: e.rows_returned,
        rows_filtered: e.rows_filtered,
        result: e.result,
        reason: e.reason,
    }
}

// =============================================================================
// Diagnostics
// =============================================================================

/// Wire → plain.
pub(crate) fn whoami_from_pb(w: pb::WhoAmIResponse) -> WhoAmI {
    WhoAmI {
        user_ulid: ulid_from_field(w.user_ulid),
        token_kind: w.token_kind,
        grant_ulid: opt_ulid_from_field(w.grant_ulid),
        grantee_label: w.grantee_label,
        caller_ip: w.caller_ip,
    }
}

/// Wire → plain.
pub(crate) fn health_from_pb(h: pb::HealthResponse) -> Health {
    Health {
        status: h.status,
        server_time_ms: h.server_time_ms,
        server_version: h.server_version,
        protocol_version: h.protocol_version,
    }
}
