//! `log_free_event` — fallback for event types without a dedicated tool.

use crate::event_json::parse_iso;
use crate::put::{ch_int, ch_real, ch_text, commit, opt_string, require_string, ts_from};
use crate::{ToolError, ToolResult};
use ohd_storage_core::events::{ChannelScalar, ChannelValue};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "log_free_event";

pub const DESCRIPTION: &str =
    "Write an event of an arbitrary type when no dedicated `log_*` tool fits. \
     Flat `data` map → one channel per key (numbers → real, integers → int, \
     bools → bool, strings → text, anything else → JSON-stringified text). \
     The storage core auto-registers unknown channels on first emit, so the \
     model is free to invent new event types (`com.user.dialysis`, …).";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "event_type":       { "type": "string" },
            "data":             { "type": "object", "description": "Flat key/value map; nested objects get JSON-stringified." },
            "timestamp":        { "type": "string" },
            "duration_seconds": { "type": "integer", "minimum": 0 },
            "notes":            { "type": "string" }
        },
        "required": ["event_type", "data"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let event_type = require_string(input, "event_type")?;
    let data = input
        .get("data")
        .and_then(|v| v.as_object())
        .ok_or_else(|| ToolError::InvalidInput("data must be an object".into()))?;

    let channels: Vec<ChannelValue> = data.iter().map(|(k, v)| {
        ChannelValue {
            channel_path: k.clone(),
            value: value_to_scalar(v),
        }
    }).collect();

    let duration_ms = input
        .get("duration_seconds")
        .and_then(|v| v.as_i64())
        .map(|s| s * 1_000);

    // Timestamp resolves either to the supplied ISO string or to "now"
    // via `ts_from`. The explicit `parse_iso` call lets us surface a
    // friendlier error when the caller misformats — but ts_from already
    // handles missing keys, so we lean on it.
    let _ = parse_iso;
    commit(
        storage,
        event_type,
        ts_from(input, "timestamp"),
        duration_ms,
        channels,
        opt_string(input, "notes"),
    )
}

fn value_to_scalar(v: &Value) -> ChannelScalar {
    match v {
        Value::Bool(b) => ChannelScalar::Bool { bool_value: *b },
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                ChannelScalar::Int { int_value: i }
            } else if let Some(f) = n.as_f64() {
                ChannelScalar::Real { real_value: f }
            } else {
                ChannelScalar::Text { text_value: n.to_string() }
            }
        }
        Value::String(s) => ChannelScalar::Text { text_value: s.clone() },
        Value::Null => ChannelScalar::Text { text_value: String::new() },
        other => ChannelScalar::Text { text_value: other.to_string() },
    }
}

// Silence unused-import warning for the helper we only kept for the
// silly-but-explicit `_ = ch_int / ch_real / ch_text` discharge above.
#[allow(unused)] use ch_int as _;
#[allow(unused)] use ch_real as _;
#[allow(unused)] use ch_text as _;
