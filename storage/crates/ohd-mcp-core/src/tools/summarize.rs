//! `summarize` — aggregate one event type into time buckets.

use crate::event_json::{ms_to_iso, now_ms, parse_iso, scalar_numeric};
use crate::{ToolError, ToolResult};
use ohd_storage_core::events::{query_events, EventFilter, EventVisibility};
use ohd_storage_core::Storage;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const NAME: &str = "summarize";

pub const DESCRIPTION: &str =
    "Aggregate events of one type into time buckets — hourly / daily / weekly / monthly. \
     Picks the first numeric channel on each event (`value`, `bpm`, `kcal`, …). Operations: \
     `avg` (default), `min`, `max`, `sum`, `count`, `median`. Default window: 90 days.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "event_type":  { "type": "string" },
            "period":      { "type": "string", "enum": ["hourly", "daily", "weekly", "monthly"], "default": "daily" },
            "aggregation": { "type": "string", "enum": ["avg", "min", "max", "sum", "count", "median"], "default": "avg" },
            "channel":     { "type": "string", "description": "Channel path to aggregate. Default: first numeric channel on each event." },
            "from_iso":    { "type": "string" },
            "to_iso":      { "type": "string" }
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
    let period = input.get("period").and_then(|v| v.as_str()).unwrap_or("daily").to_string();
    let agg = input.get("aggregation").and_then(|v| v.as_str()).unwrap_or("avg").to_string();
    let channel_filter = input.get("channel").and_then(|v| v.as_str()).map(String::from);
    let to_ms = input.get("to_iso").and_then(|v| v.as_str()).and_then(parse_iso).unwrap_or_else(now_ms);
    let from_ms = input.get("from_iso").and_then(|v| v.as_str()).and_then(parse_iso)
        .unwrap_or_else(|| to_ms - 90 * 86_400_000);

    let (events, _) = storage.with_conn(|conn| query_events(conn, &EventFilter {
        from_ms: Some(from_ms),
        to_ms: Some(to_ms),
        event_types_in: vec![event_type.clone()],
        limit: Some(10_000),
        visibility: EventVisibility::All,
        ..Default::default()
    }, None))?;

    let mut buckets: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
    for e in &events {
        let bucket_ms = bucket_floor(e.timestamp_ms, &period);
        let pick = match &channel_filter {
            Some(path) => e.channels.iter().find(|c| &c.channel_path == path),
            None => e.channels.iter().find(|c| scalar_numeric(&c.value).is_some()),
        };
        let v = pick.and_then(|c| scalar_numeric(&c.value));
        if let Some(v) = v {
            buckets.entry(bucket_ms).or_default().push(v);
        }
    }
    let out: Vec<Value> = buckets.into_iter().map(|(b, vals)| {
        let result = match agg.as_str() {
            "min" => vals.iter().cloned().fold(f64::INFINITY, f64::min),
            "max" => vals.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            "sum" => vals.iter().sum(),
            "count" => vals.len() as f64,
            "median" => median(&mut vals.clone()),
            _ => vals.iter().sum::<f64>() / vals.len() as f64,
        };
        json!({
            "bucket_iso": ms_to_iso(b),
            "bucket_ms": b,
            "value": if result.is_finite() { json!(result) } else { Value::Null },
            "n": vals.len(),
        })
    }).collect();
    Ok(json!({
        "event_type": event_type,
        "period": period,
        "aggregation": agg,
        "from_iso": ms_to_iso(from_ms),
        "to_iso": ms_to_iso(to_ms),
        "buckets": out,
    }))
}

fn bucket_floor(ts_ms: i64, period: &str) -> i64 {
    let ms = ts_ms.max(0);
    match period {
        "hourly" => (ms / 3_600_000) * 3_600_000,
        "weekly" => {
            let day = (ms / 86_400_000) * 86_400_000;
            // Align to Monday — Unix epoch (1970-01-01) was a Thursday.
            let dow = ((ms / 86_400_000) + 4) % 7; // 0 = Mon
            day - dow * 86_400_000
        }
        "monthly" => {
            // Approximate 30-day buckets — good enough for "events per month"
            // visuals without dragging in chrono for full calendar math.
            (ms / (30 * 86_400_000)) * (30 * 86_400_000)
        }
        _ => (ms / 86_400_000) * 86_400_000, // daily default
    }
}

fn median(vs: &mut [f64]) -> f64 {
    if vs.is_empty() { return 0.0; }
    vs.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = vs.len() / 2;
    if vs.len() % 2 == 0 { (vs[mid - 1] + vs[mid]) / 2.0 } else { vs[mid] }
}
