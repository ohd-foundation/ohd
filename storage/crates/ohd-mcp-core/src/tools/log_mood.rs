//! `log_mood` — mood + energy snapshot.

use crate::put::{ch_opt_text, ch_text, commit, opt_string, require_string, ts_from};
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "log_mood";

pub const DESCRIPTION: &str =
    "Log the user's current mood + energy (event_type = `wellness.mood`). \
     `mood` is free-form ('anxious', 'calm', 'irritable', 'low'). \
     `energy` is one of `low` / `moderate` / `high` when supplied.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "mood":      { "type": "string" },
            "energy":    { "type": "string", "enum": ["low", "moderate", "high"] },
            "timestamp": { "type": "string" },
            "notes":     { "type": "string" }
        },
        "required": ["mood"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let mood = require_string(input, "mood")?;
    let mut channels = vec![ch_text("mood", mood)];
    if let Some(c) = ch_opt_text("energy", opt_string(input, "energy")) {
        channels.push(c);
    }
    commit(
        storage,
        "wellness.mood".to_string(),
        ts_from(input, "timestamp"),
        None,
        channels,
        opt_string(input, "notes"),
    )
}
