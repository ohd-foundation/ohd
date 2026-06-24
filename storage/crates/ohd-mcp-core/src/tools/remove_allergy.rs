//! `remove_allergy` — mark a recorded allergy as no longer applicable.
//!
//! Writes a `profile.allergy` event with `status = "removed"` for the
//! given `fact_id` (or slug of `allergen`). The history is preserved;
//! `list_allergies` simply stops returning it.

use crate::put::{ch_text, commit, opt_string};
use crate::tools::profile_common::slug;
use crate::{ToolError, ToolResult};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "remove_allergy";

pub const DESCRIPTION: &str =
    "Remove an allergy from the user's active list. Pass `fact_id` (preferred) or \
     `allergen` (its slug is used). This records a removal — the history is kept, \
     the entry just stops appearing in the active allergy list.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "fact_id":  { "type": "string", "description": "Stable id of the allergy to remove." },
            "allergen": { "type": "string", "description": "Allergen name; its slug is used if fact_id is omitted." }
        },
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let fact_id = opt_string(input, "fact_id")
        .or_else(|| opt_string(input, "allergen").map(|a| slug(&a)))
        .ok_or_else(|| ToolError::InvalidInput("pass fact_id or allergen".into()))?;
    let allergen = opt_string(input, "allergen").unwrap_or_else(|| fact_id.clone());
    let channels = vec![
        ch_text("fact_id", fact_id),
        ch_text("allergen", allergen),
        ch_text("status", "removed".to_string()),
    ];
    commit(
        storage,
        "profile.allergy".to_string(),
        crate::event_json::now_ms(),
        None,
        channels,
        None,
    )
}
