//! Shared projection helpers for grants / pending / cases / audit rows.

use crate::event_json::{event_to_json, ms_to_iso};
use ohd_storage_core::audit::AuditEntry;
use ohd_storage_core::cases::Case;
use ohd_storage_core::grants::GrantRow;
use ohd_storage_core::pending::PendingRow;
use ohd_storage_core::ulid;
use serde_json::{json, Value};

pub fn grant_to_json(g: &GrantRow) -> Value {
    json!({
        "ulid": ulid::to_crockford(&g.ulid),
        "id": g.id,
        "grantee_label": g.grantee_label,
        "grantee_kind": g.grantee_kind,
        "purpose": g.purpose,
        "created_iso": ms_to_iso(g.created_at_ms),
        "expires_iso": g.expires_at_ms.map(ms_to_iso),
        "revoked_iso": g.revoked_at_ms.map(ms_to_iso),
        "default_action": g.default_action,
        "approval_mode": g.approval_mode,
        "aggregation_only": g.aggregation_only,
        "strip_notes": g.strip_notes,
        "notify_on_access": g.notify_on_access,
        "rolling_window_days": g.rolling_window_days,
        "max_queries_per_day": g.max_queries_per_day,
        "event_type_rules": g.event_type_rules.iter().map(|(t, e)| {
            json!({ "event_type": t, "effect": format!("{e:?}").to_lowercase() })
        }).collect::<Vec<_>>(),
        "auto_approve_event_types": g.auto_approve_event_types,
    })
}

pub fn pending_to_json(p: &PendingRow) -> Value {
    json!({
        "ulid": ulid::to_crockford(&p.ulid),
        "submitted_iso": ms_to_iso(p.submitted_at_ms),
        "submitting_grant_id": p.submitting_grant_id,
        "submitting_grant_ulid": p.submitting_grant_ulid.as_ref().map(ulid::to_crockford),
        "status": format!("{:?}", p.status).to_lowercase(),
        "reviewed_iso": p.reviewed_at_ms.map(ms_to_iso),
        "rejection_reason": p.rejection_reason,
        "expires_iso": ms_to_iso(p.expires_at_ms),
        "event": event_to_json(&p.event),
    })
}

pub fn case_to_json(c: &Case) -> Value {
    json!({
        "ulid": ulid::to_crockford(&c.ulid),
        "id": c.id,
        "case_type": c.case_type,
        "case_label": c.case_label,
        "started_iso": ms_to_iso(c.started_at_ms),
        "ended_iso": c.ended_at_ms.map(ms_to_iso),
        "active": c.ended_at_ms.is_none(),
        "parent_case_ulid": c.parent_case_ulid.as_ref().map(ulid::to_crockford),
        "predecessor_case_ulid": c.predecessor_case_ulid.as_ref().map(ulid::to_crockford),
        "opening_authority_grant_ulid": c.opening_authority_grant_ulid.as_ref().map(ulid::to_crockford),
        "last_activity_iso": ms_to_iso(c.last_activity_at_ms),
    })
}

pub fn audit_to_json(a: &AuditEntry) -> Value {
    json!({
        "ts_iso": ms_to_iso(a.ts_ms),
        "actor_type": a.actor_type,
        "auto_granted": a.auto_granted,
        "grant_id": a.grant_id,
        "action": a.action,
        "query_kind": a.query_kind,
        "rows_returned": a.rows_returned,
        "rows_filtered": a.rows_filtered,
        "result": format!("{:?}", a.result).to_lowercase(),
        "reason": a.reason,
    })
}
