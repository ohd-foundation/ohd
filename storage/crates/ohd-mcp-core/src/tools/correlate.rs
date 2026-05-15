//! `correlate` — find temporal relationships between two event types.

use crate::event_json::{event_to_json, ms_to_iso, now_ms, parse_iso, scalar_numeric};
use crate::{ToolError, ToolResult};
use ohd_storage_core::events::{query_events, EventFilter, EventVisibility};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "correlate";

pub const DESCRIPTION: &str =
    "Find events of type B occurring within `window_minutes` after each event of type A. \
     Classic example: `correlate(event_type_a=\"food.eaten\", event_type_b=\"measurement.glucose\", \
     window_minutes=180)` → post-meal glucose response. Returns per-pair rows plus summary stats \
     (count, mean delta) over the matched window.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "event_type_a":   { "type": "string", "description": "Trigger event type." },
            "event_type_b":   { "type": "string", "description": "Response event type." },
            "window_minutes": { "type": "integer", "minimum": 1, "maximum": 1440, "default": 120 },
            "from_iso":       { "type": "string" },
            "to_iso":         { "type": "string" }
        },
        "required": ["event_type_a", "event_type_b"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let a = input.get("event_type_a").and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidInput("event_type_a is required".into()))?.to_string();
    let b = input.get("event_type_b").and_then(|v| v.as_str())
        .ok_or_else(|| ToolError::InvalidInput("event_type_b is required".into()))?.to_string();
    let window_min = input.get("window_minutes").and_then(|v| v.as_i64()).unwrap_or(120).clamp(1, 1440);
    let window_ms = window_min * 60_000;
    let to_ms = input.get("to_iso").and_then(|v| v.as_str()).and_then(parse_iso).unwrap_or_else(now_ms);
    let from_ms = input.get("from_iso").and_then(|v| v.as_str()).and_then(parse_iso)
        .unwrap_or_else(|| to_ms - 30 * 86_400_000);

    let (a_events, _) = storage.with_conn(|conn| query_events(conn, &EventFilter {
        from_ms: Some(from_ms),
        to_ms: Some(to_ms),
        event_types_in: vec![a.clone()],
        limit: Some(500),
        visibility: EventVisibility::All,
        ..Default::default()
    }, None))?;
    let (b_events, _) = storage.with_conn(|conn| query_events(conn, &EventFilter {
        from_ms: Some(from_ms),
        to_ms: Some(to_ms + window_ms),
        event_types_in: vec![b.clone()],
        limit: Some(5_000),
        visibility: EventVisibility::All,
        ..Default::default()
    }, None))?;

    // For each A, take B's whose timestamp is in [A, A + window].
    // Both lists are timestamp-DESC; flip to ASC for the sweep.
    let mut a_asc = a_events.clone();
    a_asc.sort_by_key(|e| e.timestamp_ms);
    let mut b_asc = b_events.clone();
    b_asc.sort_by_key(|e| e.timestamp_ms);

    let mut pairs: Vec<Value> = Vec::with_capacity(a_asc.len());
    let mut deltas: Vec<f64> = Vec::new();
    for a_ev in &a_asc {
        let lo = a_ev.timestamp_ms;
        let hi = lo + window_ms;
        let bs: Vec<&_> = b_asc.iter().filter(|b| b.timestamp_ms >= lo && b.timestamp_ms <= hi).collect();
        let mut b_jsons: Vec<Value> = Vec::with_capacity(bs.len());
        for b_ev in &bs {
            let dt_min = (b_ev.timestamp_ms - a_ev.timestamp_ms) as f64 / 60_000.0;
            deltas.push(dt_min);
            let mut row = event_to_json(b_ev);
            if let Value::Object(ref mut m) = row {
                m.insert("delta_minutes_from_a".into(), json!(dt_min));
            }
            b_jsons.push(row);
        }
        let a_first_numeric = a_ev.channels.iter().find_map(|c| scalar_numeric(&c.value));
        pairs.push(json!({
            "a": event_to_json(a_ev),
            "a_numeric": a_first_numeric,
            "b_count": bs.len(),
            "b_events": b_jsons,
        }));
    }
    let mean_delta = if deltas.is_empty() { None } else {
        Some(deltas.iter().sum::<f64>() / deltas.len() as f64)
    };
    Ok(json!({
        "event_type_a": a,
        "event_type_b": b,
        "window_minutes": window_min,
        "from_iso": ms_to_iso(from_ms),
        "to_iso": ms_to_iso(to_ms),
        "a_count": a_asc.len(),
        "b_total_in_pairs": deltas.len(),
        "mean_delta_minutes": mean_delta,
        "pairs": pairs,
    }))
}
