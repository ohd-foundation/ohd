//! `log_medication` — record a dose actually taken / skipped / late / refused.

use crate::put::{ch_opt_real, ch_opt_text, ch_text, commit, opt_string, require_string, ts_from};
use crate::{ToolError, ToolResult};
use ohd_storage_core::events::{ChannelScalar, ChannelValue};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "log_medication";

pub const DESCRIPTION: &str =
    "Log a medication dose (event type `medication.taken`). Record the dose the user \
     ACTUALLY took in `dose_value` + `dose_unit` — this may differ from what was \
     prescribed, and capturing the real value is what matters. Do NOT adjust the \
     reported dose to match a prescription. `status` is `taken` (default), `skipped`, \
     `late`, or `refused`; a skipped dose is clinically important data, not a failure \
     to hide. `regimen_id` links this dose to an active regimen (see \
     list_active_regimens). `adherence_reason` is optional context the user \
     volunteered (e.g. \"nauseous\", \"ran out\") — never prompt for it.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name":             { "type": "string", "description": "Medication name, e.g. 'metformin'." },
            "regimen_id":       { "type": "string", "description": "Links to an active regimen (optional)." },
            "dose_value":       { "type": "number", "description": "The dose ACTUALLY taken (not the prescribed dose)." },
            "dose_unit":        { "type": "string", "description": "Dose unit: 'mg', 'tablets', 'ml'." },
            "dose_note":        { "type": "string", "description": "Free-text dose qualifier, e.g. 'half a tablet'." },
            "status":           { "type": "string", "enum": ["taken", "skipped", "late", "refused"], "default": "taken" },
            "adherence_reason": { "type": "string", "description": "Optional context the user volunteered." },
            "timestamp":        { "type": "string", "description": "ISO 8601; defaults to now." },
            "notes":            { "type": "string" }
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

    let mut channels = vec![ch_text("name", name), ch_text("status", status.clone())];
    if let Some(c) = ch_opt_text("regimen_id", opt_string(input, "regimen_id")) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_real("dose_value", input.get("dose_value").and_then(|v| v.as_f64())) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("dose_unit", opt_string(input, "dose_unit")) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("dose_note", opt_string(input, "dose_note")) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("adherence_reason", opt_string(input, "adherence_reason")) {
        channels.push(c);
    }
    // A skip carries an explicit `skipped = true` bool channel in addition
    // to status, so adherence queries can filter on it without string
    // matching — and so a missed dose is a first-class recorded fact, not
    // an absence to be inferred.
    if status == "skipped" {
        channels.push(ChannelValue {
            channel_path: "skipped".to_string(),
            value: ChannelScalar::Bool { bool_value: true },
        });
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
