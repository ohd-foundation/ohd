//! `log_sleep` — bedtime → wake_time session.

use crate::event_json::parse_iso;
use crate::put::{ch_int, ch_opt_text, ch_text, commit, opt_string, ts_from};
use crate::{ToolError, ToolResult};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "log_sleep";

pub const DESCRIPTION: &str =
    "Log a sleep session (event_type = `activity.sleep`). Both `bedtime` and \
     `wake_time` are required ISO 8601 timestamps. The event's stored \
     `duration_minutes` is derived (wake − bedtime). `quality` is one of \
     `poor` / `fair` / `good` / `great` when supplied.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "bedtime":   { "type": "string", "description": "ISO 8601." },
            "wake_time": { "type": "string", "description": "ISO 8601." },
            "quality":   { "type": "string", "enum": ["poor", "fair", "good", "great"] },
            "notes":     { "type": "string" }
        },
        "required": ["bedtime", "wake_time"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let bedtime_ms = parse_iso(
        input
            .get("bedtime")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("bedtime is required".into()))?,
    )
    .ok_or_else(|| ToolError::InvalidInput("bedtime must be valid ISO 8601".into()))?;
    let wake_ms = parse_iso(
        input
            .get("wake_time")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidInput("wake_time is required".into()))?,
    )
    .ok_or_else(|| ToolError::InvalidInput("wake_time must be valid ISO 8601".into()))?;
    if wake_ms <= bedtime_ms {
        return Err(ToolError::InvalidInput(
            "wake_time must be after bedtime".into(),
        ));
    }
    let duration_min = (wake_ms - bedtime_ms) / 60_000;

    let mut channels = vec![ch_int("duration_minutes", duration_min)];
    channels.push(ch_text("bedtime_iso", crate::event_json::ms_to_iso(bedtime_ms)));
    channels.push(ch_text("wake_iso", crate::event_json::ms_to_iso(wake_ms)));
    if let Some(c) = ch_opt_text("quality", opt_string(input, "quality")) {
        channels.push(c);
    }
    // `ts_from` returns now() if neither bedtime nor parse_iso is wired,
    // but we already validated above. Re-use bedtime as the event ts.
    let _ = ts_from;
    commit(
        storage,
        "activity.sleep".to_string(),
        bedtime_ms,
        Some(wake_ms - bedtime_ms),
        channels,
        opt_string(input, "notes"),
    )
}
