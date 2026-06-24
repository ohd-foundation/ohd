//! Shared helpers for the clinical-case + clinical-event tools.
//!
//! Clinical events (`clinical.visit` / `clinical.prescription` /
//! `clinical.lab_result`) belong to a *case* (a clinical episode). Cases
//! link to events through case-filters, not a foreign key — so we use
//! the correlation_id precedent: every clinical event carries a `case_id`
//! text channel = the case ULID, and the case gets one filter with a
//! channel-predicate `{case_id eq <ulid>}`. That makes both
//! `get_case_timeline` (a direct channel-predicate query) and
//! grant-scope resolution (`compute_case_scope`) return exactly the
//! tagged events.

use crate::{ToolError, ToolResult};
use ohd_storage_core::cases::{add_case_filter, case_id_by_ulid};
use ohd_storage_core::events::{ChannelPredicate, ChannelScalar, EventFilter};
use ohd_storage_core::{ulid, Storage};

/// Resolve a crockford case ULID string to its storage rowid.
pub fn case_rowid(storage: &Storage, case_ulid: &str) -> ToolResult<i64> {
    let parsed = ulid::parse_crockford(case_ulid)
        .map_err(|_| ToolError::InvalidInput(format!("invalid case_ulid: {case_ulid}")))?;
    storage
        .with_conn(|conn| case_id_by_ulid(conn, &parsed))
        .map_err(|_| ToolError::InvalidInput(format!("case not found: {case_ulid}")))
}

/// An EventFilter that matches every event tagged with this case_id.
pub fn case_member_filter(case_ulid: &str) -> EventFilter {
    EventFilter {
        channel_predicates: vec![ChannelPredicate {
            channel_path: "case_id".to_string(),
            op: "eq".to_string(),
            value: ChannelScalar::Text { text_value: case_ulid.to_string() },
        }],
        ..Default::default()
    }
}

/// Register the `{case_id eq <ulid>}` filter on a case so its scope (and
/// thus any grant sharing it) includes the tagged events. Best-effort:
/// the event is already tagged with the case_id channel, so a failure
/// here doesn't lose the linkage for direct queries.
pub fn attach_case_filter(storage: &Storage, case_ulid: &str) -> ToolResult<()> {
    let rowid = case_rowid(storage, case_ulid)?;
    let filter = case_member_filter(case_ulid);
    storage.with_conn_mut(|conn| {
        add_case_filter(conn, rowid, &filter, Some("case members"), None)
    })?;
    Ok(())
}
