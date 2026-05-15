//! `force_close_case` — user-initiated case close. Revokes the opening
//! authority's grant if one is attached.

use crate::event_json::ms_to_iso;
use crate::{ToolError, ToolResult};
use ohd_storage_core::cases::{close_case, list_cases, ListCasesFilter};
use ohd_storage_core::grants::{grant_id_by_ulid, revoke_grant};
use ohd_storage_core::ulid;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "force_close_case";

pub const DESCRIPTION: &str =
    "Close one case by ULID. If the case has an opening authority grant \
     attached (paramedic break-glass, clinician visit, …) the grant is \
     revoked synchronously. Use sparingly — case close is normally driven \
     by the authority's own session ending.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "case_ulid": { "type": "string" },
            "reason":    { "type": "string" }
        },
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
    let reason = input.get("reason").and_then(|v| v.as_str()).map(String::from);

    let cases = storage.with_conn(|conn| list_cases(conn, &ListCasesFilter {
        include_closed: false,
        limit: Some(1000),
        ..Default::default()
    }))?;
    let case = cases
        .into_iter()
        .find(|c| ulid::random_tail(&c.ulid) == target_tail)
        .ok_or_else(|| ToolError::InvalidInput("case not found or already closed".into()))?;
    let authority_ulid = case.opening_authority_grant_ulid.clone();

    let (closed_case, _reopen_token) = storage.with_conn_mut(|conn| {
        close_case(conn, case.id, None, false, None)
    })?;
    let closed_at = closed_case.ended_at_ms.unwrap_or(0);
    let revoked: Option<i64> = if let Some(g_ulid) = authority_ulid {
        storage.with_conn_mut(|conn| {
            let gid = grant_id_by_ulid(conn, &g_ulid)?;
            revoke_grant(conn, gid, reason.as_deref()).map(Some)
        })?
    } else { None };

    Ok(json!({
        "ok": true,
        "case_ulid": raw,
        "closed_iso": ms_to_iso(closed_at),
        "authority_grant_revoked_at_ms": revoked,
    }))
}
