//! `discontinue_medication_regimen` — end an active medication course.

use crate::put::{ch_opt_text, ch_text, commit, opt_string, require_string};
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "discontinue_medication_regimen";

pub const DESCRIPTION: &str =
    "End a medication regimen. Pass the `regimen_id` (from start_medication_regimen \
     or list_active_regimens) and optionally a `reason`. The regimen stops appearing \
     in list_active_regimens; its history and any logged doses are preserved.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "regimen_id": { "type": "string", "description": "The regimen to discontinue." },
            "reason":     { "type": "string", "description": "Why it was stopped, optional." },
            "ended":      { "type": "string", "description": "ISO 8601; defaults to now." }
        },
        "required": ["regimen_id"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let regimen_id = require_string(input, "regimen_id")?;
    let mut channels = vec![ch_text("regimen_id", regimen_id)];
    if let Some(c) = ch_opt_text("reason", opt_string(input, "reason")) {
        channels.push(c);
    }
    commit(
        storage,
        "medication.regimen_discontinued".to_string(),
        crate::put::ts_from(input, "ended"),
        None,
        channels,
        None,
    )
}
