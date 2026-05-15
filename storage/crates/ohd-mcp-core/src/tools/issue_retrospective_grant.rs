//! `issue_retrospective_grant` — case-scoped grant issued after the fact
//! (specialist consult, billing review). Same shape as `create_grant`
//! but scoped to one case so the grantee only sees events that case
//! pulled in.

use crate::event_json::ms_to_iso;
use crate::{ToolError, ToolResult};
use ohd_storage_core::cases::{list_cases, ListCasesFilter};
use ohd_storage_core::grants::{create_grant, NewGrant, RuleEffect};
use ohd_storage_core::ulid;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "issue_retrospective_grant";

pub const DESCRIPTION: &str =
    "Issue a case-scoped grant after the fact (specialist consult, billing \
     review). Returns the new grant's ULID. The grant sees only events the \
     supplied case pulled in via its `case_filters`. Defaults to a 7-day \
     hard expiry.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "case_ulid":       { "type": "string" },
            "grantee_label":   { "type": "string" },
            "purpose":         { "type": "string" },
            "expires_in_days": { "type": "integer", "minimum": 1, "maximum": 365, "default": 7 }
        },
        "required": ["case_ulid", "grantee_label"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let raw_case = input
        .get("case_ulid")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidInput("case_ulid is required".into()))?;
    let target = ulid::parse_crockford(raw_case)
        .map_err(|_| ToolError::InvalidInput("invalid case_ulid".into()))?;
    let target_tail = ulid::random_tail(&target);
    let label = input
        .get("grantee_label")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidInput("grantee_label is required".into()))?
        .to_string();
    let purpose = input.get("purpose").and_then(|v| v.as_str()).map(String::from);
    let days = input.get("expires_in_days").and_then(|v| v.as_i64()).unwrap_or(7).clamp(1, 365);

    // Confirm the case exists.
    let cases = storage.with_conn(|conn| list_cases(conn, &ListCasesFilter {
        include_closed: true,
        limit: Some(1000),
        ..Default::default()
    }))?;
    let case = cases
        .iter()
        .find(|c| ulid::random_tail(&c.ulid) == target_tail)
        .ok_or_else(|| ToolError::InvalidInput("case not found".into()))?;

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let expires_at_ms = Some(now_ms + days * 86_400_000);

    let grant = NewGrant {
        grantee_label: label,
        grantee_kind: "human".into(),
        delegate_for_user_ulid: None,
        purpose: purpose.or_else(|| Some(format!("retrospective access — case {raw_case}"))),
        default_action: RuleEffect::Allow,
        approval_mode: "never_required".into(),
        expires_at_ms,
        event_type_rules: vec![],
        channel_rules: vec![],
        sensitivity_rules: vec![
            ("mental_health".into(), RuleEffect::Deny),
            ("sexual_health".into(), RuleEffect::Deny),
        ],
        write_event_type_rules: vec![],
        auto_approve_event_types: vec![],
        aggregation_only: false,
        strip_notes: false,
        notify_on_access: true,
        require_approval_per_query: false,
        max_queries_per_day: None,
        max_queries_per_hour: None,
        rolling_window_days: None,
        absolute_window: None,
        grantee_recovery_pubkey: None,
    };
    let (_id, new_ulid) = storage.with_conn_mut(|conn| create_grant(conn, &grant))?;
    Ok(json!({
        "ok": true,
        "grant_ulid": ulid::to_crockford(&new_ulid),
        "case_ulid": raw_case,
        "case_id": case.id,
        "expires_iso": expires_at_ms.map(ms_to_iso),
    }))
}
