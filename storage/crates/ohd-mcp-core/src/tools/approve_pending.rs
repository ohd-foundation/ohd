//! `approve_pending` — promote one pending event into `events`.

use crate::event_json::ms_to_iso;
use crate::{ToolError, ToolResult};
use ohd_storage_core::pending::approve_pending;
use ohd_storage_core::ulid;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "approve_pending";

pub const DESCRIPTION: &str =
    "Approve one pending event by its ULID. The row is promoted to `events` \
     with the same ULID. When `also_trust_event_type = true`, the event's \
     type is added to the submitting grant's auto-approve allowlist so \
     future writes of the same type skip the queue.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "pending_ulid":          { "type": "string" },
            "also_trust_event_type": { "type": "boolean", "default": false }
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
    let trust = input.get("also_trust_event_type").and_then(|v| v.as_bool()).unwrap_or(false);
    let envelope_key = storage.envelope_key().cloned();
    let (committed_at_ms, event_ulid) = storage.with_conn_mut(|conn| {
        approve_pending(conn, &pending_ulid, trust, envelope_key.as_ref())
    })?;
    Ok(json!({
        "ok": true,
        "event_ulid": ulid::to_crockford(&event_ulid),
        "committed_iso": ms_to_iso(committed_at_ms),
    }))
}
