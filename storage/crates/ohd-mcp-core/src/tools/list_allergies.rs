//! `list_allergies` — the user's current active allergies.

use crate::tools::profile_common::active_facts;
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "list_allergies";

pub const DESCRIPTION: &str =
    "List the user's current allergies (active only — removed ones are omitted). \
     Each entry carries allergen, severity, reaction, and a stable fact_id.";

pub fn input_schema() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

pub fn execute(_input: &Value, storage: &Storage) -> ToolResult<Value> {
    let allergies = active_facts(storage, "profile.allergy", &["removed"])?;
    Ok(json!({ "count": allergies.len(), "allergies": allergies }))
}
