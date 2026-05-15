//! `chart` — return the data for a chart description.
//!
//! Stubbed today: doesn't render an image, just returns the time-series
//! the user described (event type + window). The model can describe the
//! chart in prose using the underlying data and tell the user to inspect
//! it. Full chart rendering moves into a future iteration.

use crate::event_json::{ms_to_iso, now_ms, parse_iso, scalar_numeric};
use crate::{ToolError, ToolResult};
use ohd_storage_core::events::{query_events, EventFilter, EventVisibility};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "chart";

pub const DESCRIPTION: &str =
    "Return the underlying time-series for a chart description. Specify `event_type` and \
     optionally a `channel` to pick which numeric channel. Defaults to the last 14 days. \
     Result is a list of `{ts_iso, value}` points suitable for the model to summarise verbally.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "event_type": { "type": "string" },
            "channel":    { "type": "string", "description": "Channel path; default = first numeric channel." },
            "from_iso":   { "type": "string" },
            "to_iso":     { "type": "string" }
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
    let channel_filter = input.get("channel").and_then(|v| v.as_str()).map(String::from);
    let to_ms = input.get("to_iso").and_then(|v| v.as_str()).and_then(parse_iso).unwrap_or_else(now_ms);
    let from_ms = input.get("from_iso").and_then(|v| v.as_str()).and_then(parse_iso)
        .unwrap_or_else(|| to_ms - 14 * 86_400_000);

    let (mut events, _) = storage.with_conn(|conn| query_events(conn, &EventFilter {
        from_ms: Some(from_ms),
        to_ms: Some(to_ms),
        event_types_in: vec![event_type.clone()],
        limit: Some(2_000),
        visibility: EventVisibility::All,
        ..Default::default()
    }, None))?;
    events.sort_by_key(|e| e.timestamp_ms);

    let points: Vec<Value> = events.iter().filter_map(|e| {
        let v = match &channel_filter {
            Some(p) => e.channels.iter().find(|c| &c.channel_path == p).and_then(|c| scalar_numeric(&c.value)),
            None => e.channels.iter().find_map(|c| scalar_numeric(&c.value)),
        }?;
        Some(json!({
            "ts_iso": ms_to_iso(e.timestamp_ms),
            "ts_ms": e.timestamp_ms,
            "value": v,
        }))
    }).collect();
    Ok(json!({
        "event_type": event_type,
        "channel": channel_filter,
        "from_iso": ms_to_iso(from_ms),
        "to_iso": ms_to_iso(to_ms),
        "point_count": points.len(),
        "points": points,
    }))
}
