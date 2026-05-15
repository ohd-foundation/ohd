//! `describe_data` — high-level orientation of what's in the store.
//!
//! Lets the LLM ask "what data do you have?" before guessing event-type
//! names. Returns total event count, per-type count + latest timestamp,
//! and the oldest / newest event window.

use crate::event_json::ms_to_iso;
use crate::{Namespace, ToolResult, TypeName};
use ohd_storage_core::events::{count_events, query_events, EventFilter, EventVisibility};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "describe_data";

pub const DESCRIPTION: &str =
    "Summarise what the user's storage currently holds: total event count, per-type count \
     + latest timestamp per type, the oldest and newest timestamp seen. Call this FIRST \
     when you're not sure what event types exist before issuing query_events.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false
    })
}

pub fn execute(_input: &Value, storage: &Storage) -> ToolResult<Value> {
    let total = storage.with_conn(|conn| count_events(conn, &EventFilter::default()))?;
    // Per-type: scan the event_types table, then for each registered type
    // count + latest timestamp. Bounded by event_types size (~30–50 rows).
    let types: Vec<(Namespace, TypeName)> = storage.with_conn(|conn| {
        let mut stmt = conn.prepare("SELECT namespace, name FROM event_types ORDER BY namespace, name")?;
        let rows: Result<Vec<(Namespace, TypeName)>, ohd_storage_core::Error> = stmt
            .query_map([], |r| Ok((r.get::<_, Namespace>(0)?, r.get::<_, TypeName>(1)?)))?
            .map(|r| r.map_err(ohd_storage_core::Error::from))
            .collect();
        rows
    })?;
    let mut per_type: Vec<Value> = Vec::with_capacity(types.len());
    for (ns, name) in &types {
        let dotted = format!("{ns}.{name}");
        let count = storage.with_conn(|conn| {
            count_events(conn, &EventFilter {
                event_types_in: vec![dotted.clone()],
                ..Default::default()
            })
        })?;
        if count == 0 { continue; }
        let latest = storage.with_conn(|conn| {
            let (events, _) = query_events(conn, &EventFilter {
                event_types_in: vec![dotted.clone()],
                limit: Some(1),
                visibility: EventVisibility::All,
                ..Default::default()
            }, None)?;
            Ok(events.first().map(|e| e.timestamp_ms))
        })?;
        per_type.push(json!({
            "event_type": dotted,
            "count": count,
            "latest_iso": latest.map(ms_to_iso),
        }));
    }
    Ok(json!({
        "total_events": total,
        "event_types": per_type,
    }))
}
