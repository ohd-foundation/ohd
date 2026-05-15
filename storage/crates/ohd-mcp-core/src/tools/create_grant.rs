//! `create_grant` — issue a grant from a named template.
//!
//! Pragmatic port of the Python tool: the agent picks a template id and a
//! grantee label, and we build a sensible [`NewGrant`] under the hood
//! with defaults that mirror the in-app templates. Custom rules
//! (per-channel, per-sensitivity) stay in the in-app grant editor — the
//! agent doesn't need that level of control to do useful things.

use crate::event_json::ms_to_iso;
use crate::{ToolError, ToolResult};
use ohd_storage_core::grants::{create_grant, NewGrant, RuleEffect};
use ohd_storage_core::ulid;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "create_grant";

pub const DESCRIPTION: &str =
    "Issue a grant from a named template. Returns the grant ULID + creation \
     time. Templates: `primary_doctor` (read all measurements, log medications), \
     `specialist_visit` (last 30 days, read-only), `emergency_template` \
     (read essentials), `researcher` (aggregation-only). For anything more \
     specific the user should open the in-app grant editor.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "template_id":     { "type": "string", "enum": ["primary_doctor", "specialist_visit", "emergency_template", "researcher"] },
            "grantee_label":   { "type": "string" },
            "purpose":         { "type": "string" },
            "expires_in_days": { "type": "integer", "minimum": 1, "maximum": 3650 }
        },
        "required": ["template_id", "grantee_label"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let tpl = input
        .get("template_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidInput("template_id is required".into()))?
        .to_string();
    let label = input
        .get("grantee_label")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidInput("grantee_label is required".into()))?
        .to_string();
    let purpose = input.get("purpose").and_then(|v| v.as_str()).map(String::from);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let expires_at_ms = input
        .get("expires_in_days")
        .and_then(|v| v.as_i64())
        .map(|d| now_ms + d * 86_400_000);

    let mut grant = NewGrant {
        grantee_label: label,
        grantee_kind: "human".into(),
        delegate_for_user_ulid: None,
        purpose,
        default_action: RuleEffect::Deny,
        approval_mode: "always".into(),
        expires_at_ms,
        event_type_rules: vec![],
        channel_rules: vec![],
        sensitivity_rules: vec![],
        write_event_type_rules: vec![],
        auto_approve_event_types: vec![],
        aggregation_only: false,
        strip_notes: false,
        notify_on_access: false,
        require_approval_per_query: false,
        max_queries_per_day: None,
        max_queries_per_hour: None,
        rolling_window_days: None,
        absolute_window: None,
        grantee_recovery_pubkey: None,
    };
    match tpl.as_str() {
        "primary_doctor" => {
            grant.default_action = RuleEffect::Allow;
            grant.sensitivity_rules = vec![
                ("mental_health".into(), RuleEffect::Deny),
                ("sexual_health".into(), RuleEffect::Deny),
            ];
            grant.write_event_type_rules = vec![
                ("medication.taken".into(), RuleEffect::Allow),
                ("std.clinical_note".into(), RuleEffect::Allow),
            ];
            grant.approval_mode = "auto_for_event_types".into();
        }
        "specialist_visit" => {
            grant.default_action = RuleEffect::Allow;
            grant.rolling_window_days = Some(30);
            grant.sensitivity_rules = vec![
                ("mental_health".into(), RuleEffect::Deny),
                ("sexual_health".into(), RuleEffect::Deny),
            ];
        }
        "emergency_template" => {
            grant.event_type_rules = vec![
                ("measurement.blood_pressure".into(), RuleEffect::Allow),
                ("measurement.heart_rate".into(), RuleEffect::Allow),
                ("measurement.glucose".into(), RuleEffect::Allow),
                ("measurement.spo2".into(), RuleEffect::Allow),
                ("medication.taken".into(), RuleEffect::Allow),
                ("profile.allergy".into(), RuleEffect::Allow),
                ("profile.condition".into(), RuleEffect::Allow),
            ];
        }
        "researcher" => {
            grant.default_action = RuleEffect::Allow;
            grant.aggregation_only = true;
            grant.strip_notes = true;
            grant.sensitivity_rules = vec![
                ("mental_health".into(), RuleEffect::Deny),
                ("sexual_health".into(), RuleEffect::Deny),
                ("substance_use".into(), RuleEffect::Deny),
            ];
        }
        other => return Err(ToolError::InvalidInput(format!("unknown template_id: {other}"))),
    }

    let (_id, new_ulid) = storage.with_conn_mut(|conn| create_grant(conn, &grant))?;
    Ok(json!({
        "ok": true,
        "grant_ulid": ulid::to_crockford(&new_ulid),
        "template_id": tpl,
        "expires_iso": expires_at_ms.map(ms_to_iso),
    }))
}
