//! `reject_pending` — mark one pending event rejected.

use crate::event_json::ms_to_iso;
use crate::{ToolError, ToolResult};
use ohd_storage_core::pending::reject_pending;
use ohd_storage_core::ulid;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "reject_pending";

pub const DESCRIPTION: &str =
    "Reject one pending event by its ULID. Marks the row `rejected` with the \
     supplied `reason` (free text). The submitting grantee sees a \
     `REJECTED` outcome on their next sync.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pending_ulid": { "type": "string" },
            "reason":       { "type": "string" }
        },
        "required": ["pending_ulid"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let raw = input
        .get("pending_ulid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidInput("pending_ulid is required".into()))?;
    let pending_ulid = ulid::parse_crockford(raw).map_err(|_| ToolError::InvalidInput("invalid ULID".into()))?;
    let reason = input.get("reason").and_then(|v| v.as_str()).map(String::from);
    let rejected_at_ms = storage.with_conn_mut(|conn| {
        reject_pending(conn, &pending_ulid, reason.as_deref())
    })?;
    Ok(json!({
        "ok": true,
        "pending_ulid": raw,
        "rejected_iso": ms_to_iso(rejected_at_ms),
    }))
}
