//! `list_grants` — active grants the user has issued.

use crate::grant_json::grant_to_json;
use crate::ToolResult;
use ohd_storage_core::grants::{list_grants, ListGrantsFilter};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "list_grants";

pub const DESCRIPTION: &str =
    "List grants the user has issued (clinicians, devices, agents). Defaults \
     to active only — set `include_revoked = true` to see ones the user already \
     pulled.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "include_revoked": { "type": "boolean", "default": false },
            "grantee_kind":    { "type": "string", "description": "Filter by 'human' / 'app' / 'service' / 'emergency' / …" }
        },
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let include_revoked = input.get("include_revoked").and_then(|v| v.as_bool()).unwrap_or(false);
    let grantee_kind = input.get("grantee_kind").and_then(|v| v.as_str()).map(String::from);
    let filter = ListGrantsFilter {
        include_revoked,
        include_expired: false,
        grantee_kind,
        only_grant_id: None,
        limit: Some(200),
    };
    let rows = storage.with_conn(|conn| list_grants(conn, &filter))?;
    let json_rows: Vec<Value> = rows.iter().map(grant_to_json).collect();
    Ok(json!({ "count": json_rows.len(), "grants": json_rows }))
}
