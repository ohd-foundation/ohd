//! `list_cases` — active and recently-closed cases.

use crate::grant_json::case_to_json;
use crate::ToolResult;
use ohd_storage_core::cases::{list_cases, ListCasesFilter};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "list_cases";

pub const DESCRIPTION: &str =
    "List clinical cases — emergencies, admissions, visits. Active first, \
     then recently closed. Pass `include_closed = false` to hide everything \
     that's already ended.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "include_closed": { "type": "boolean", "default": true }
        },
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let include_closed = input.get("include_closed").and_then(|v| v.as_bool()).unwrap_or(true);
    let filter = ListCasesFilter {
        include_closed,
        limit: Some(200),
        ..Default::default()
    };
    let cases = storage.with_conn(|conn| list_cases(conn, &filter))?;
    let out: Vec<Value> = cases.iter().map(case_to_json).collect();
    Ok(json!({ "count": out.len(), "cases": out }))
}
