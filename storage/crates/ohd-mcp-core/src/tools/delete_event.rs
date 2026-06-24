//! `delete_event` — hard-delete a single event by ULID.
//!
//! People and agents make mistakes; this is the "remove that wrong entry"
//! escape hatch. It's owner-only (ToolKind::Operator → never exposed to a
//! share) and irreversible: the row and its channels are gone (the row's
//! own audit trail goes with it, but other events are untouched). For a
//! correction that should preserve history, edit/supersede instead.

use crate::{ToolError, ToolResult};
use ohd_storage_core::events::{hard_delete_events, DeleteFilter};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "delete_event";

pub const DESCRIPTION: &str =
    "Permanently delete one or more events by their ULID — the escape hatch for a \
     mistaken entry (a dose that wasn't taken, a wrong measurement, a duplicate). \
     Pass `ulid` (a single ULID) or `ulids` (a list). This is irreversible and \
     removes the event entirely; to fix a value while keeping history, log a \
     correction instead. Owner-only.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "ulid":  { "type": "string", "description": "The ULID of the event to delete." },
            "ulids": { "type": "array", "items": { "type": "string" }, "description": "Several ULIDs to delete." }
        },
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let mut ulids: Vec<String> = Vec::new();
    if let Some(u) = input.get("ulid").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        ulids.push(u.to_string());
    }
    if let Some(arr) = input.get("ulids").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str().filter(|s| !s.is_empty()) {
                ulids.push(s.to_string());
            }
        }
    }
    if ulids.is_empty() {
        return Err(ToolError::InvalidInput("ulid or ulids is required".into()));
    }

    let filter = DeleteFilter { event_ulids: ulids.clone(), ..Default::default() };
    let deleted = storage
        .with_conn_mut(|conn| hard_delete_events(conn, &filter))
        .map_err(|e| ToolError::Internal(e.to_string()))?;

    Ok(json!({ "ok": true, "deleted": deleted, "ulids": ulids }))
}
