//! `list_pending` — writes awaiting user review.

use crate::grant_json::pending_to_json;
use crate::ToolResult;
use ohd_storage_core::pending::{list_pending, ListPendingFilter};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "list_pending";

pub const DESCRIPTION: &str =
    "List pending writes awaiting the user's review. Each entry is one event \
     a grantee submitted that hit the approval queue. Pass `limit` (max 1000) \
     to bound the result.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "limit":  { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100 },
            "status": { "type": "string", "enum": ["pending", "approved", "rejected", "expired"] }
        },
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let limit = input.get("limit").and_then(|v| v.as_i64()).unwrap_or(100).clamp(1, 1000);
    let filter = ListPendingFilter {
        submitting_grant_id: None,
        status: input.get("status").and_then(|v| v.as_str()).and_then(parse_status),
        limit: Some(limit),
    };
    let rows = storage.with_conn(|conn| list_pending(conn, &filter))?;
    let out: Vec<Value> = rows.iter().map(pending_to_json).collect();
    Ok(json!({ "count": out.len(), "pending": out }))
}

fn parse_status(s: &str) -> Option<ohd_storage_core::pending::PendingStatus> {
    use ohd_storage_core::pending::PendingStatus;
    match s {
        "pending" => Some(PendingStatus::Pending),
        "approved" => Some(PendingStatus::Approved),
        "rejected" => Some(PendingStatus::Rejected),
        "expired" => Some(PendingStatus::Expired),
        _ => None,
    }
}
