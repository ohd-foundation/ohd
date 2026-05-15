//! `log_symptom` — record a symptom the user is experiencing right now.

use crate::put::{ch_opt_text, ch_real, commit, opt_string, require_string, ts_from};
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "log_symptom";

pub const DESCRIPTION: &str =
    "Log a symptom the user is currently experiencing. The event type is \
     `symptom.<slug>` — the slug is derived from the `symptom` parameter \
     (lowercase, non-alphanumeric → `_`). Defaults to the current moment.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "symptom":  { "type": "string", "description": "Symptom name, e.g. 'headache', 'nausea'." },
            "severity": { "type": "number", "description": "Numeric severity, 0–10. Optional." },
            "severity_label": { "type": "string", "description": "Qualitative severity: 'mild' / 'moderate' / 'severe'." },
            "location": { "type": "string", "description": "Anatomical location, e.g. 'frontal', 'left knee'." },
            "notes":    { "type": "string" },
            "timestamp": { "type": "string", "description": "ISO 8601; defaults to now." }
        },
        "required": ["symptom"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let symptom = require_string(input, "symptom")?;
    let slug = slugify(&symptom);
    let event_type = format!("symptom.{slug}");

    let mut channels = Vec::new();
    if let Some(s) = input.get("severity").and_then(|v| v.as_f64()) {
        channels.push(ch_real("severity", s));
    }
    if let Some(c) = ch_opt_text("severity_label", opt_string(input, "severity_label")) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("location", opt_string(input, "location")) {
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

fn slugify(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}
