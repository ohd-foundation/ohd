//! `list_conditions` — the user's current active conditions.

use crate::tools::profile_common::active_facts;
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "list_conditions";

pub const DESCRIPTION: &str =
    "List the user's current medical conditions (active only — resolved ones omitted). \
     Each entry carries name, optional icd10, onset, and a stable fact_id.";

pub fn input_schema() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

pub fn execute(_input: &Value, storage: &Storage) -> ToolResult<Value> {
    let conditions = active_facts(storage, "profile.condition", &["resolved"])?;
    Ok(json!({ "count": conditions.len(), "conditions": conditions }))
}
