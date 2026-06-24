//! `record_condition` — add or update a diagnosed condition.

use crate::put::{ch_opt_int, ch_opt_text, ch_text, commit, opt_string, require_string};
use crate::tools::profile_common::slug;
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "record_condition";

pub const DESCRIPTION: &str =
    "Record a medical condition / diagnosis the user has. Pass `name` (required, \
     e.g. \"type 2 diabetes\"), and optionally `icd10` (code), `onset_iso` (when it \
     began), and a stable `fact_id` (defaults to slug(name) — reuse to update). \
     Recorded as active; use resolve_condition when it ends.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name":      { "type": "string", "description": "Condition / diagnosis name." },
            "icd10":     { "type": "string", "description": "ICD-10 code, optional." },
            "onset_iso": { "type": "string", "description": "ISO 8601 onset date, optional." },
            "fact_id":   { "type": "string", "description": "Stable id; defaults to slug(name)." }
        },
        "required": ["name"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let name = require_string(input, "name")?;
    let fact_id = opt_string(input, "fact_id").unwrap_or_else(|| slug(&name));
    let onset_ms = input
        .get("onset_iso")
        .and_then(|v| v.as_str())
        .and_then(crate::event_json::parse_iso);
    let mut channels = vec![
        ch_text("fact_id", fact_id),
        ch_text("name", name),
        ch_text("status", "active".to_string()),
    ];
    if let Some(c) = ch_opt_text("icd10", opt_string(input, "icd10")) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_int("onset_ms", onset_ms) {
        channels.push(c);
    }
    commit(
        storage,
        "profile.condition".to_string(),
        crate::event_json::now_ms(),
        None,
        channels,
        None,
    )
}
