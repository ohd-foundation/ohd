//! `log_exercise` — manual workout entry.

use crate::put::{ch_opt_int, ch_opt_text, ch_text, commit, opt_string, require_string, ts_from};
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "log_exercise";

pub const DESCRIPTION: &str =
    "Log an exercise session (event_type = `activity.exercise_session`). \
     Pass `activity` (free-form: 'running', 'cycling', 'yoga') and optionally \
     `duration_minutes`, `intensity` ('low' / 'moderate' / 'high'), ISO 8601 \
     `started`, and `notes`. Use this only for manual entries — Health Connect \
     sync writes the same event type for watch-recorded sessions.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "activity":         { "type": "string" },
            "duration_minutes": { "type": "integer", "minimum": 0 },
            "intensity":        { "type": "string", "enum": ["low", "moderate", "high"] },
            "started":          { "type": "string" },
            "notes":            { "type": "string" }
        },
        "required": ["activity"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let activity = require_string(input, "activity")?;
    let duration_minutes = input.get("duration_minutes").and_then(|v| v.as_i64());

    let mut channels = vec![ch_text("title", activity)];
    if let Some(c) = ch_opt_int("duration_minutes", duration_minutes) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("intensity", opt_string(input, "intensity")) {
        channels.push(c);
    }
    commit(
        storage,
        "activity.exercise_session".to_string(),
        ts_from(input, "started"),
        duration_minutes.map(|m| m * 60_000),
        channels,
        opt_string(input, "notes"),
    )
}
