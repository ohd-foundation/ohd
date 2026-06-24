//! `list_measurement_watches` — the measurements the user is tracking.

use crate::tools::watch_common::active_watches;
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "list_measurement_watches";

pub const DESCRIPTION: &str =
    "List the measurements the user is currently tracking (started and not stopped). \
     Each carries watch_id, metric, label, schedule, on_hand, quick, and the \
     ordering case_id when known. Readings are logged with log_measurement.";

pub fn input_schema() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

pub fn execute(_input: &Value, storage: &Storage) -> ToolResult<Value> {
    let watches = active_watches(storage)?;
    Ok(json!({ "count": watches.len(), "watches": watches }))
}
