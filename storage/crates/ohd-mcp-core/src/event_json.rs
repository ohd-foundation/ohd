//! Shared projection helpers — Event → JSON for tool replies.
//!
//! Every read-side tool surfaces events with the same flat shape so the
//! LLM doesn't have to learn N different schemas. Keep this module the
//! only place we convert between storage `Event` and tool-output JSON.

use ohd_storage_core::events::{ChannelScalar, ChannelValue, Event};
use serde_json::{json, Map, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// Render one Event as JSON. Channels become a flat `{path: primitive}`
/// map — Real → number, Int → number, Bool → bool, Text → string,
/// EnumOrdinal → integer.
pub fn event_to_json(e: &Event) -> Value {
    let mut channels = Map::new();
    for c in &e.channels {
        channels.insert(c.channel_path.clone(), scalar_to_json(&c.value));
    }
    json!({
        "ulid": e.ulid,
        "ts_ms": e.timestamp_ms,
        "ts_iso": ms_to_iso(e.timestamp_ms),
        "duration_ms": e.duration_ms,
        "event_type": e.event_type,
        "channels": Value::Object(channels),
        "notes": e.notes,
        "source": e.source,
        "top_level": e.top_level,
    })
}

/// One channel scalar → primitive JSON.
pub fn channel_to_json(c: &ChannelValue) -> (String, Value) {
    (c.channel_path.clone(), scalar_to_json(&c.value))
}

pub fn scalar_to_json(s: &ChannelScalar) -> Value {
    match s {
        ChannelScalar::Real { real_value } => json!(real_value),
        ChannelScalar::Int { int_value } => json!(int_value),
        ChannelScalar::Bool { bool_value } => json!(bool_value),
        ChannelScalar::Text { text_value } => json!(text_value),
        ChannelScalar::EnumOrdinal { enum_ordinal } => json!(enum_ordinal),
    }
}

/// Pull a `Real` (or coerced `Int`) out of a channel for aggregation /
/// correlation. Returns `None` for Text / Bool / Enum.
pub fn scalar_numeric(s: &ChannelScalar) -> Option<f64> {
    match s {
        ChannelScalar::Real { real_value } => Some(*real_value),
        ChannelScalar::Int { int_value } => Some(*int_value as f64),
        _ => None,
    }
}

/// Format a Unix-ms timestamp as RFC 3339 UTC. Falls back to the epoch
/// string on any conversion failure (e.g. timestamps past year 9999).
pub fn ms_to_iso(ts_ms: i64) -> String {
    let nanos = (ts_ms as i128) * 1_000_000;
    let dt = match OffsetDateTime::from_unix_timestamp_nanos(nanos) {
        Ok(dt) => dt,
        Err(_) => return "1970-01-01T00:00:00Z".to_string(),
    };
    dt.format(&Rfc3339).unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Parse an ISO 8601 / RFC 3339 string into Unix-ms. Returns `None` on
/// any parse failure (caller falls back to a default window).
pub fn parse_iso(raw: &str) -> Option<i64> {
    OffsetDateTime::parse(raw, &Rfc3339)
        .ok()
        .map(|dt| (dt.unix_timestamp_nanos() / 1_000_000) as i64)
}

/// "now" as Unix-ms — used as the default `to_iso` upper bound.
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
