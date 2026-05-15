//! Shared write-path helpers for the `log_*` tools.
//!
//! Every `log_*` tool ends with the same shape: take a small set of
//! parameters, build one [`EventInput`], call `put_events`, return the
//! committed ULID. This module factors out the boilerplate so the tool
//! files stay focused on the JSON-input → EventInput projection.

use crate::event_json::{ms_to_iso, now_ms, parse_iso};
use crate::{EventType, ToolError, ToolResult};
use ohd_storage_core::events::{
    put_events, ChannelScalar, ChannelValue, EventInput, PutEventResult,
};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

/// Source tag every `log_*` tool stamps on the event so a future
/// `audit_query` can attribute writes back to "the agent did this".
pub const SOURCE_TAG: &str = "cord_mcp";

/// Resolve the `timestamp` / `started` / `bedtime` field from the input
/// JSON — caller picks which key to read. Falls back to "now" when the
/// key is missing or the ISO parse fails.
pub fn ts_from(input: &Value, key: &str) -> i64 {
    input
        .get(key)
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .unwrap_or_else(now_ms)
}

/// Pull a string field, lower-bound length 1. Used for required text
/// arguments (`event_type`, `name`, `description`).
pub fn require_string(input: &Value, key: &str) -> ToolResult<String> {
    input
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .ok_or_else(|| ToolError::InvalidInput(format!("{key} is required")))
}

/// Pull an optional string field. Empty strings collapse to `None` so
/// the model doesn't have to differentiate "missing" vs "empty".
pub fn opt_string(input: &Value, key: &str) -> Option<String> {
    input
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// One channel entry builder — keeps the call sites short.
pub fn ch_text(path: &str, v: String) -> ChannelValue {
    ChannelValue {
        channel_path: path.to_string(),
        value: ChannelScalar::Text { text_value: v },
    }
}

pub fn ch_real(path: &str, v: f64) -> ChannelValue {
    ChannelValue {
        channel_path: path.to_string(),
        value: ChannelScalar::Real { real_value: v },
    }
}

pub fn ch_int(path: &str, v: i64) -> ChannelValue {
    ChannelValue {
        channel_path: path.to_string(),
        value: ChannelScalar::Int { int_value: v },
    }
}

/// Skip nulls so the event ends up sparse — dynamic channel registration
/// only kicks in for channels we actually emit. Cleaner storage.
pub fn ch_opt_text(path: &str, v: Option<String>) -> Option<ChannelValue> {
    v.map(|s| ch_text(path, s))
}

pub fn ch_opt_real(path: &str, v: Option<f64>) -> Option<ChannelValue> {
    v.map(|x| ch_real(path, x))
}

pub fn ch_opt_int(path: &str, v: Option<i64>) -> Option<ChannelValue> {
    v.map(|x| ch_int(path, x))
}

/// Write one event and return the JSON the tool surface ships back.
pub fn commit(
    storage: &Storage,
    event_type: EventType,
    timestamp_ms: i64,
    duration_ms: Option<i64>,
    channels: Vec<ChannelValue>,
    notes: Option<String>,
) -> ToolResult<Value> {
    let input = EventInput {
        timestamp_ms,
        duration_ms,
        event_type: event_type.clone(),
        channels,
        source: Some(SOURCE_TAG.to_string()),
        notes,
        ..Default::default()
    };
    let envelope_key = storage.envelope_key().cloned();
    let results = storage.with_conn_mut(|conn| {
        put_events(conn, &[input], None, false, envelope_key.as_ref())
    })?;
    let first = results
        .into_iter()
        .next()
        .ok_or_else(|| ToolError::Internal("put_events returned no result".into()))?;
    let out = match first {
        PutEventResult::Committed { ulid, committed_at_ms } => json!({
            "ok": true,
            "ulid": ulid,
            "event_type": event_type,
            "timestamp_ms": timestamp_ms,
            "timestamp_iso": ms_to_iso(timestamp_ms),
            "committed_at_iso": ms_to_iso(committed_at_ms),
        }),
        PutEventResult::Pending { ulid, expires_at_ms } => json!({
            "ok": true,
            "pending": true,
            "ulid": ulid,
            "event_type": event_type,
            "expires_at_iso": ms_to_iso(expires_at_ms),
        }),
        PutEventResult::Error { code, message } => json!({
            "ok": false,
            "error_code": code,
            "error": message,
        }),
    };
    Ok(out)
}
