//! `log_medication` — record a dose taken / skipped / late / refused.

use crate::put::{ch_opt_real, ch_opt_text, ch_text, commit, opt_string, require_string, ts_from};
use crate::{ToolError, ToolResult};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "log_medication";

pub const DESCRIPTION: &str =
    "Log a medication dose. Event type = `medication.taken`. `status` is \
     one of `taken` (default), `skipped`, `late`, `refused`. `dose` parses as \
     a number (mg, tablets) — pass the unit as a separate field.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name":      { "type": "string", "description": "Medication name, e.g. 'metformin'." },
            "dose":      { "type": "number" },
            "unit":      { "type": "string", "description": "Dose unit: 'mg', 'tablets', 'ml'." },
            "status":    { "type": "string", "enum": ["taken", "skipped", "late", "refused"], "default": "taken" },
            "timestamp": { "type": "string", "description": "ISO 8601; defaults to now." },
            "notes":     { "type": "string" }
        },
        "required": ["name"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let name = require_string(input, "name")?;
    let status = input.get("status").and_then(|v| v.as_str()).unwrap_or("taken").to_string();
    if !matches!(status.as_str(), "taken" | "skipped" | "late" | "refused") {
        return Err(ToolError::InvalidInput(format!("invalid status: {status}")));
    }

    let mut channels = vec![ch_text("name", name), ch_text("status", status)];
    if let Some(c) = ch_opt_real("dose", input.get("dose").and_then(|v| v.as_f64())) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("unit", opt_string(input, "unit")) {
        channels.push(c);
    }
    commit(
        storage,
        "medication.taken".to_string(),
        ts_from(input, "timestamp"),
        None,
        channels,
        opt_string(input, "notes"),
    )
}
