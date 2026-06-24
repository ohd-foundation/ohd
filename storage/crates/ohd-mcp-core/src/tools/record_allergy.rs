//! `record_allergy` — add or update a recorded allergy.
//!
//! Writes a `profile.allergy` event with a stable `fact_id` (slug of the
//! allergen unless one is supplied) and `status = "active"`. Recording
//! the same allergen again updates it (latest-per-fact-id wins).

use crate::put::{ch_opt_text, ch_text, commit, opt_string, require_string};
use crate::tools::profile_common::slug;
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "record_allergy";

pub const DESCRIPTION: &str =
    "Record an allergy the user has. Pass `allergen` (required, e.g. \"penicillin\"), \
     and optionally `severity` (mild | moderate | severe | unknown), `reaction` \
     (free text, e.g. \"hives\"), and a stable `fact_id` (defaults to a slug of the \
     allergen — pass the same one to update an existing entry). Recording an allergen \
     that already exists updates it in place.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "allergen": { "type": "string", "description": "What the user is allergic to." },
            "severity": { "type": "string", "enum": ["mild", "moderate", "severe", "unknown"], "default": "unknown" },
            "reaction": { "type": "string", "description": "Observed reaction, optional." },
            "fact_id":  { "type": "string", "description": "Stable id; defaults to slug(allergen)." }
        },
        "required": ["allergen"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let allergen = require_string(input, "allergen")?;
    let fact_id = opt_string(input, "fact_id").unwrap_or_else(|| slug(&allergen));
    let severity = opt_string(input, "severity").unwrap_or_else(|| "unknown".to_string());
    let mut channels = vec![
        ch_text("fact_id", fact_id),
        ch_text("allergen", allergen),
        ch_text("severity", severity),
        ch_text("status", "active".to_string()),
    ];
    if let Some(c) = ch_opt_text("reaction", opt_string(input, "reaction")) {
        channels.push(c);
    }
    commit(
        storage,
        "profile.allergy".to_string(),
        crate::event_json::now_ms(),
        None,
        channels,
        None,
    )
}
