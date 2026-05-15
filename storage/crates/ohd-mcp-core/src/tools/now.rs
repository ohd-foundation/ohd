//! `now` — current wall-clock time + system timezone.
//!
//! Sounds trivial but in practice the LLM constantly needs to ground
//! "this week" / "yesterday" / "tonight" against a fixed reference. Better
//! one canonical tool that always returns the same shape than a system
//! prompt that re-derives the answer per turn.

use crate::{ToolError, ToolResult};
use ohd_storage_core::Storage;
use serde_json::{json, Value};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

pub const NAME: &str = "now";

pub const DESCRIPTION: &str =
    "Return the current wall-clock time (ISO 8601 UTC) and the system timezone id. \
     Use this to resolve relative time references like 'today', 'yesterday', 'this week' \
     before issuing query_events with explicit timestamps.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {},
        "additionalProperties": false,
    })
}

pub fn execute(_input: &Value, _storage: &Storage) -> ToolResult<Value> {
    let now = OffsetDateTime::now_utc();
    let iso = now
        .format(&Rfc3339)
        .map_err(|e| ToolError::Internal(e.to_string()))?;
    let ts_ms = (now.unix_timestamp_nanos() / 1_000_000) as i64;
    let tz = std::env::var("TZ").unwrap_or_else(|_| "UTC".to_string());
    Ok(json!({
        "iso": iso,
        "timestamp_ms": ts_ms,
        "tz": tz,
    }))
}
