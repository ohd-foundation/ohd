//! `resolve_condition` — mark a condition as resolved (no longer active).

use crate::put::{ch_text, commit, opt_string};
use crate::tools::profile_common::slug;
use crate::{ToolError, ToolResult};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "resolve_condition";

pub const DESCRIPTION: &str =
    "Mark a condition as resolved (recovered / no longer active). Pass `fact_id` \
     (preferred) or `name` (its slug is used). History is preserved; the condition \
     stops appearing in the active list.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "fact_id": { "type": "string", "description": "Stable id of the condition." },
            "name":    { "type": "string", "description": "Condition name; slug used if fact_id omitted." }
        },
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let fact_id = opt_string(input, "fact_id")
        .or_else(|| opt_string(input, "name").map(|n| slug(&n)))
        .ok_or_else(|| ToolError::InvalidInput("pass fact_id or name".into()))?;
    let name = opt_string(input, "name").unwrap_or_else(|| fact_id.clone());
    let channels = vec![
        ch_text("fact_id", fact_id),
        ch_text("name", name),
        ch_text("status", "resolved".to_string()),
    ];
    commit(
        storage,
        "profile.condition".to_string(),
        crate::event_json::now_ms(),
        None,
        channels,
        None,
    )
}
