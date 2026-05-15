//! `revoke_grant` — pull a grant. Synchronous; the next call from the
//! grantee fails with `TOKEN_REVOKED`.

use crate::put::SOURCE_TAG;
use crate::{ToolError, ToolResult};
use ohd_storage_core::grants::{grant_id_by_ulid, revoke_grant};
use ohd_storage_core::ulid;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "revoke_grant";

pub const DESCRIPTION: &str =
    "Revoke a grant by its ULID. Synchronous — the grantee's next call \
     denies. Pass an optional `reason` (free text) for the audit log.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "grant_ulid": { "type": "string", "description": "Crockford-base32 grant ULID." },
            "reason":     { "type": "string" }
        },
        "required": ["grant_ulid"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let raw = input
        .get("grant_ulid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidInput("grant_ulid is required".into()))?;
    let ulid = ulid::parse_crockford(raw).map_err(|_| ToolError::InvalidInput("invalid grant_ulid".into()))?;
    let reason = input.get("reason").and_then(|v| v.as_str()).map(String::from);

    let revoked_at_ms = storage.with_conn(|conn| {
        let grant_id = grant_id_by_ulid(conn, &ulid)?;
        revoke_grant(conn, grant_id, reason.as_deref())
    })?;
    let _ = SOURCE_TAG;
    Ok(json!({
        "ok": true,
        "grant_ulid": raw,
        "revoked_at_ms": revoked_at_ms,
    }))
}
