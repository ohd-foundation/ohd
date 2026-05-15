//! `audit_query` — per-grant or global audit view.

use crate::event_json::{now_ms, parse_iso};
use crate::grant_json::audit_to_json;
use crate::{ToolError, ToolResult};
use ohd_storage_core::audit::{query, AuditQuery};
use ohd_storage_core::grants::grant_id_by_ulid;
use ohd_storage_core::ulid;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "audit_query";

pub const DESCRIPTION: &str =
    "Per-grant or global audit view. Each row carries `auto_granted` so the \
     UI / model can flag emergency-timeout entries distinctly. Pass \
     `grant_ulid` to filter to one grant's history; otherwise returns the \
     global audit log.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "grant_ulid": { "type": "string", "description": "Crockford grant ULID; omit for global view." },
            "from_iso":   { "type": "string" },
            "to_iso":     { "type": "string" },
            "actor_type": { "type": "string", "description": "Filter by actor type ('self', 'grant', 'emergency', …)." },
            "action":     { "type": "string", "description": "'read' / 'write' / 'revoke' / 'create_grant' / …" },
            "limit":      { "type": "integer", "minimum": 1, "maximum": 10000, "default": 500 }
        },
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let to_ms = input.get("to_iso").and_then(|v| v.as_str()).and_then(parse_iso).unwrap_or_else(now_ms);
    let from_ms = input.get("from_iso").and_then(|v| v.as_str()).and_then(parse_iso);
    let limit = input.get("limit").and_then(|v| v.as_i64()).unwrap_or(500).clamp(1, 10_000);

    let grant_id = match input.get("grant_ulid").and_then(|v| v.as_str()) {
        Some(raw) => {
            let g = ulid::parse_crockford(raw)
                .map_err(|_| ToolError::InvalidInput("invalid grant_ulid".into()))?;
            Some(storage.with_conn(|conn| grant_id_by_ulid(conn, &g))?)
        }
        None => None,
    };
    let q = AuditQuery {
        from_ms,
        to_ms: Some(to_ms),
        grant_id,
        actor_type: input.get("actor_type").and_then(|v| v.as_str()).map(String::from),
        action: input.get("action").and_then(|v| v.as_str()).map(String::from),
        result: None,
        limit: Some(limit),
    };
    let rows = storage.with_conn(|conn| query(conn, &q))?;
    let out: Vec<Value> = rows.iter().map(audit_to_json).collect();
    Ok(json!({ "count": out.len(), "entries": out }))
}
