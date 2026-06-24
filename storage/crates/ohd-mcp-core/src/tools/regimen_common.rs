//! Shared helper for medication-regimen projection.
//!
//! A regimen is a `medication.regimen_started` event carrying a minted
//! `regimen_id`; ending it writes a `medication.regimen_discontinued`
//! with the same `regimen_id`. Active regimens = started whose
//! `regimen_id` has no matching discontinued event (same shape as the
//! correlation sweep in `correlate.rs`). Used by `list_active_regimens`
//! and `get_health_profile`.

use crate::tools::profile_common::{channel_text, flatten_event, query_all};
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::Value;
use std::collections::HashSet;

/// Active regimens, newest first, as flat JSON (channels hoisted).
pub fn active_regimens(storage: &Storage) -> ToolResult<Vec<Value>> {
    let started = query_all(storage, "medication.regimen_started")?;
    let discontinued = query_all(storage, "medication.regimen_discontinued")?;
    let stopped: HashSet<String> = discontinued
        .iter()
        .filter_map(|e| channel_text(e, "regimen_id"))
        .collect();
    // regimen_id is a freshly minted ULID per start, so each started
    // event is a distinct regimen — no dedup needed, just drop stopped.
    let mut out: Vec<Value> = started
        .iter()
        .filter(|e| {
            channel_text(e, "regimen_id")
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
