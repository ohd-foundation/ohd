//! Shared helper for measurement-watch projection.
//!
//! A watch is a `measurement.watch_started` event carrying a minted
//! `watch_id`; stopping it writes a `measurement.watch_stopped` with the
//! same `watch_id`. Active watches = started whose `watch_id` has no
//! matching stopped event — the same shape as `regimen_common::active_regimens`.
//! Used by `list_measurement_watches` and `get_treatment_plan`.

use crate::tools::profile_common::{channel_text, flatten_event, query_all};
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::Value;
use std::collections::HashSet;

/// Active measurement watches, newest first, as flat JSON (channels hoisted).
pub fn active_watches(storage: &Storage) -> ToolResult<Vec<Value>> {
    let started = query_all(storage, "measurement.watch_started")?;
    let stopped_events = query_all(storage, "measurement.watch_stopped")?;
    let stopped: HashSet<String> = stopped_events
        .iter()
        .filter_map(|e| channel_text(e, "watch_id"))
        .collect();
    let mut out: Vec<Value> = started
        .iter()
        .filter(|e| {
            channel_text(e, "watch_id")
                .map(|id| !stopped.contains(&id))
                .unwrap_or(false)
        })
        .map(flatten_event)
        .collect();
    out.sort_by(|a, b| {
        b.get("ts_ms").and_then(Value::as_i64).cmp(&a.get("ts_ms").and_then(Value::as_i64))
    });
    Ok(out)
}
