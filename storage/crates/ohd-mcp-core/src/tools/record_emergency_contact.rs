//! `record_emergency_contact` — add or update an emergency contact.

use crate::put::{ch_opt_text, ch_text, commit, opt_string, require_string};
use crate::tools::profile_common::slug;
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "record_emergency_contact";

pub const DESCRIPTION: &str =
    "Record an emergency contact. Pass `name` (required) and `phone`, optionally \
     `relation` (e.g. \"spouse\", \"mother\") and a stable `fact_id` (defaults to \
     slug(name) — reuse to update).";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name":     { "type": "string", "description": "Contact's name." },
            "phone":    { "type": "string", "description": "Contact phone number." },
            "relation": { "type": "string", "description": "Relationship to the user." },
            "fact_id":  { "type": "string", "description": "Stable id; defaults to slug(name)." }
        },
        "required": ["name"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let name = require_string(input, "name")?;
    let fact_id = opt_string(input, "fact_id").unwrap_or_else(|| slug(&name));
    let mut channels = vec![
        ch_text("fact_id", fact_id),
        ch_text("name", name),
        ch_text("status", "active".to_string()),
    ];
    if let Some(c) = ch_opt_text("phone", opt_string(input, "phone")) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("relation", opt_string(input, "relation")) {
        channels.push(c);
    }
    commit(
        storage,
        "profile.emergency_contact".to_string(),
        crate::event_json::now_ms(),
        None,
        channels,
        None,
    )
}
