//! `query_events` — the workhorse read tool.

use crate::event_json::{event_to_json, ms_to_iso, now_ms, parse_iso};
use crate::{ToolError, ToolResult};
use ohd_storage_core::events::{query_events, EventFilter, EventVisibility};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "query_events";

pub const DESCRIPTION: &str =
    "Read the user's events with optional filters. Supports an exact event-type match \
     (`event_type`) or a prefix (`event_type_prefix`, e.g. `intake.` to pull every intake.*). \
     Defaults to the last 30 days, top-level rows only, capped at 100 results. Pass \
     `visibility = \"all\"` to include detail / sample rows the timeline normally hides.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "event_type":        { "type": "string", "description": "Exact dotted event type, e.g. `food.eaten`." },
            "event_type_prefix": { "type": "string", "description": "Match every event type starting with this string." },
            "from_iso":          { "type": "string", "description": "ISO 8601 lower bound. Default: 30 days ago." },
            "to_iso":            { "type": "string", "description": "ISO 8601 upper bound. Default: now." },
            "limit":             { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100 },
            "order":             { "type": "string", "enum": ["asc", "desc"], "default": "desc" },
            "visibility":        { "type": "string", "enum": ["top_level", "non_top_level", "all"], "default": "top_level" },
            "source":            { "type": "string", "description": "Restrict to events whose `source` equals this string." }
        },
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let to_ms = input.get("to_iso").and_then(|v| v.as_str()).and_then(parse_iso).unwrap_or_else(now_ms);
    let from_ms = input
        .get("from_iso")
        .and_then(|v| v.as_str())
        .and_then(parse_iso)
        .unwrap_or_else(|| to_ms - 30 * 86_400_000);
    let limit = input.get("limit").and_then(|v| v.as_i64()).unwrap_or(100).clamp(1, 1000);
    let order = input.get("order").and_then(|v| v.as_str()).unwrap_or("desc");
    let visibility = match input.get("visibility").and_then(|v| v.as_str()) {
        Some("non_top_level") => EventVisibility::NonTopLevelOnly,
        Some("all") => EventVisibility::All,
        _ => EventVisibility::TopLevelOnly,
    };
    let source_in: Vec<String> = input
        .get("source")
        .and_then(|v| v.as_str())
        .map(|s| vec![s.to_string()])
        .unwrap_or_default();

    let exact = input.get("event_type").and_then(|v| v.as_str()).map(String::from);
    let prefix = input.get("event_type_prefix").and_then(|v| v.as_str()).map(String::from);
    if exact.is_some() && prefix.is_some() {
        return Err(ToolError::InvalidInput(
            "pass either event_type OR event_type_prefix, not both".into(),
        ));
    }
    let event_types_in: Vec<String> = if let Some(name) = exact {
        vec![name]
    } else if let Some(pfx) = prefix {
        // event_types is small (~50 rows). Direct SQL scan is fine — we
        // don't have a registry helper for prefix matching yet.
        storage.with_conn(|conn| {
            // Match both canonical rows (`<prefix>.*`) and their custom
            // shadows (`custom.<prefix>.*`) so unpromoted user-extended types
            // are transparently included in prefix queries. The reader never
            // needs to know which side a particular type lives on.
            let mut stmt = conn.prepare(
                "SELECT namespace, name FROM event_types
                 WHERE namespace || '.' || name LIKE ?1 || '%'
                    OR (namespace = 'custom' AND name LIKE ?1 || '%')
                 ORDER BY namespace, name",
            )?;
            let rows: Result<Vec<String>, ohd_storage_core::Error> = stmt
                .query_map([&pfx], |r| {
                    let ns: String = r.get(0)?;
                    let n: String = r.get(1)?;
                    Ok(format!("{ns}.{n}"))
                })?
                .map(|r| r.map_err(ohd_storage_core::Error::from))
                .collect();
            rows
        })?
    } else {
        vec![]
    };

    let (mut events, _) = storage.with_conn(|conn| {
        query_events(conn, &EventFilter {
            from_ms: Some(from_ms),
            to_ms: Some(to_ms),
            event_types_in,
            limit: Some(limit),
            visibility,
            source_in,
            ..Default::default()
        }, None)
    })?;
    if order == "asc" {
        events.reverse();
    }
    let rows: Vec<Value> = events.iter().map(event_to_json).collect();
    Ok(json!({
        "from_iso": ms_to_iso(from_ms),
        "to_iso": ms_to_iso(to_ms),
        "count": rows.len(),
        "events": rows,
    }))
}
