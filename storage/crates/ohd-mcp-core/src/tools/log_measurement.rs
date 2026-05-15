//! `log_measurement` — generic single-value measurement.

use crate::put::{ch_opt_text, ch_real, ch_text, commit, opt_string, require_string, ts_from};
use crate::{ToolError, ToolResult};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "log_measurement";

pub const DESCRIPTION: &str =
    "Log a generic numeric measurement when no dedicated tool exists. The \
     `event_type` is namespaced — pass `measurement.<slug>` for known shapes \
     (`measurement.glucose`, `measurement.heart_rate`, `measurement.body_fat`) \
     or a free string for custom metrics (`measurement.urine.glucose`). \
     Dynamic channel registration handles new types automatically.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "event_type": { "type": "string", "description": "Dotted event type, e.g. `measurement.glucose`." },
            "value":      { "type": "number" },
            "unit":       { "type": "string", "description": "Unit, e.g. 'mmol/L', 'mmHg', '°C'." },
            "timestamp":  { "type": "string" },
            "notes":      { "type": "string" }
        },
        "required": ["event_type", "value"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let event_type = require_string(input, "event_type")?;
    let value = input
        .get("value")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| ToolError::InvalidInput("value (number) is required".into()))?;
    let unit = require_string(input, "unit").ok();

    let mut channels = vec![ch_real("value", value)];
    if let Some(u) = unit {
        channels.push(ch_text("unit", u));
    }
    if let Some(c) = ch_opt_text("notes", opt_string(input, "notes")) {
        channels.push(c);
    }
    commit(
        storage,
        event_type,
        ts_from(input, "timestamp"),
        None,
        channels,
        opt_string(input, "notes"),
    )
}
