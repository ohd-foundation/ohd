//! `get_medications_taken` — adherence view.

use crate::event_json::{event_to_json, ms_to_iso, now_ms, parse_iso};
use crate::ToolResult;
use ohd_storage_core::events::{query_events, ChannelScalar, EventFilter};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "get_medications_taken";

pub const DESCRIPTION: &str =
    "Return medication adherence — every `medication.taken` event in the window. Defaults \
     to the last 30 days. Filter by `medication_name` (case-insensitive) to pull just one \
     drug's history.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "from_iso":        { "type": "string" },
            "to_iso":          { "type": "string" },
            "medication_name": { "type": "string" }
        },
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let to_ms = input.get("to_iso").and_then(|v| v.as_str()).and_then(parse_iso).unwrap_or_else(now_ms);
    let from_ms = input.get("from_iso").and_then(|v| v.as_str()).and_then(parse_iso)
        .unwrap_or_else(|| to_ms - 30 * 86_400_000);
    let name_filter = input.get("medication_name").and_then(|v| v.as_str()).map(|s| s.to_lowercase());

    let (events, _) = storage.with_conn(|conn| query_events(conn, &EventFilter {
        from_ms: Some(from_ms),
        to_ms: Some(to_ms),
        event_types_in: vec!["medication.taken".into()],
        limit: Some(1_000),
        ..Default::default()
    }, None))?;

    let rows: Vec<Value> = events.iter().filter(|e| match &name_filter {
        None => true,
        Some(needle) => e.channels.iter().any(|c| {
            (c.channel_path == "name" || c.channel_path == "med.name")
                && matches!(&c.value, ChannelScalar::Text { text_value } if text_value.to_lowercase() == *needle)
        }),
    }).map(event_to_json).collect();
    Ok(json!({
        "from_iso": ms_to_iso(from_ms),
        "to_iso": ms_to_iso(to_ms),
        "count": rows.len(),
        "events": rows,
    }))
}
