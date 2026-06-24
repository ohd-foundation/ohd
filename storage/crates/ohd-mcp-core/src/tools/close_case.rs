//! `close_case` — end a clinical case.

use crate::tools::clinical_common::case_rowid;
use crate::{ToolError, ToolResult};
use ohd_storage_core::cases::close_case;
use crate::grant_json::case_to_json;
use crate::put::require_string;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "close_case";

pub const DESCRIPTION: &str =
    "Close a clinical case (mark the episode ended). Pass the `case_ulid`. The case \
     and its events are preserved; it just moves to the closed list. Events recorded \
     after close still belong to the case only if it is reopened.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "case_ulid": { "type": "string", "description": "The case to close." }
        },
        "required": ["case_ulid"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let case_ulid = require_string(input, "case_ulid")?;
    let rowid = case_rowid(storage, &case_ulid)?;
    let (case, _token) = storage
        .with_conn_mut(|conn| close_case(conn, rowid, None, false, None))
        .map_err(|e| ToolError::Internal(format!("close_case: {e}")))?;
    Ok(json!({ "ok": true, "case": case_to_json(&case) }))
}
