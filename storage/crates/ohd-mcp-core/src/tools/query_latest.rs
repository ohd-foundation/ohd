//! `query_latest` — top-N latest events of a specific type.

use crate::event_json::event_to_json;
use crate::{ToolError, ToolResult};
use ohd_storage_core::events::{query_events, EventFilter, EventVisibility};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "query_latest";

pub const DESCRIPTION: &str =
    "Return the most recent N events of a given type. Cheap when you only need the latest \
     reading (e.g. `measurement.heart_rate`, count=1) — much faster than `query_events` with \
     a wide window.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "event_type": { "type": "string" },
            "count":      { "type": "integer", "minimum": 1, "maximum": 100, "default": 1 }
        },
        "required": ["event_type"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let event_type = input
        .get("event_type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidInput("event_type is required".into()))?
        .to_string();
    let count = input.get("count").and_then(|v| v.as_i64()).unwrap_or(1).clamp(1, 100);

    let (events, _) = storage.with_conn(|conn| {
        query_events(conn, &EventFilter {
            event_types_in: vec![event_type.clone()],
            limit: Some(count),
            visibility: EventVisibility::All,
            ..Default::default()
        }, None)
    })?;
    let rows: Vec<Value> = events.iter().map(event_to_json).collect();
    Ok(json!({
        "event_type": event_type,
        "count": rows.len(),
        "events": rows,
    }))
}
