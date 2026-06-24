//! `get_case_timeline` — every event tagged with a case's id.

use crate::event_json::event_to_json;
use crate::put::require_string;
use crate::tools::clinical_common::case_member_filter;
use crate::ToolResult;
use ohd_storage_core::events::{query_events, EventVisibility};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "get_case_timeline";

pub const DESCRIPTION: &str =
    "Return every event belonging to a clinical case (visits, prescriptions, lab \
     results, and anything else tagged with this case_id), newest first. Pass the \
     `case_ulid`. This is the 'what happened during this episode' view.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "case_ulid": { "type": "string", "description": "The case to read." }
        },
        "required": ["case_ulid"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let case_ulid = require_string(input, "case_ulid")?;
    let mut filter = case_member_filter(&case_ulid);
    filter.limit = Some(1_000);
    filter.visibility = EventVisibility::All;
    let (events, _) = storage.with_conn(|conn| query_events(conn, &filter, None))?;
    let rows: Vec<Value> = events.iter().map(event_to_json).collect();
    Ok(json!({
        "case_ulid": case_ulid,
        "count": rows.len(),
        "events": rows,
    }))
}
