//! `stop_measurement_watch` — stop tracking a measurement.

use crate::put::{ch_opt_text, ch_text, commit, opt_string, require_string};
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "stop_measurement_watch";

pub const DESCRIPTION: &str =
    "Stop a measurement watch. Pass the `watch_id` (from start_measurement_watch \
     or list_measurement_watches) and optionally a `reason`. The watch stops \
     appearing in list_measurement_watches; past readings are preserved.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "watch_id": { "type": "string", "description": "The watch to stop." },
            "reason":   { "type": "string", "description": "Why it was stopped, optional." },
            "ended":    { "type": "string", "description": "ISO 8601; defaults to now." }
        },
        "required": ["watch_id"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let watch_id = require_string(input, "watch_id")?;
    let mut channels = vec![ch_text("watch_id", watch_id)];
    if let Some(c) = ch_opt_text("reason", opt_string(input, "reason")) {
        channels.push(c);
    }
    commit(
        storage,
        "measurement.watch_stopped".to_string(),
        crate::put::ts_from(input, "ended"),
        None,
        channels,
        None,
    )
}
