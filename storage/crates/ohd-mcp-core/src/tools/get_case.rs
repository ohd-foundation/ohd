//! `get_case` — return one case row by ULID.

use crate::grant_json::case_to_json;
use crate::{ToolError, ToolResult};
use ohd_storage_core::cases::{list_cases, ListCasesFilter};
use ohd_storage_core::ulid;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "get_case";

pub const DESCRIPTION: &str =
    "Return one case by ULID. Includes case_type, label, start / end times, \
     parent / predecessor links, and the opening authority's grant ULID. For \
     the full event timeline scoped to this case, use `query_events` with \
     `case_ulid` once that filter ships.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": { "case_ulid": { "type": "string" } },
        "required": ["case_ulid"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let raw = input
        .get("case_ulid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidInput("case_ulid is required".into()))?;
    let target = ulid::parse_crockford(raw)
        .map_err(|_| ToolError::InvalidInput("invalid case_ulid".into()))?;
    let target_tail = ulid::random_tail(&target);

    // list_cases is the public read API; ULID lookup would deserve a
    // dedicated helper but isn't critical-path. Scan a bounded set and
    // pick the matching row.
    let cases = storage.with_conn(|conn| list_cases(conn, &ListCasesFilter {
        include_closed: true,
        limit: Some(1000),
        ..Default::default()
    }))?;
    let found = cases.iter().find(|c| ulid::random_tail(&c.ulid) == target_tail);
    match found {
        Some(c) => Ok(json!({ "ok": true, "case": case_to_json(c) })),
        None => Ok(json!({ "ok": false, "error": "case not found" })),
    }
}
