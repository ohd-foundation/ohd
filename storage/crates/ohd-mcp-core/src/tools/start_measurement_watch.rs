//! `start_measurement_watch` — begin tracking a measurement on a cadence.
//!
//! Mints a `watch_id` and writes a `measurement.watch_started` event. The
//! actual readings are ordinary `measurement.*` events; the watch only
//! declares intent + schedule ("watch my temperature daily"). Stop it with
//! `stop_measurement_watch`.

use crate::put::{ch_opt_bool, ch_opt_text, ch_text, commit, opt_string, require_string};
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "start_measurement_watch";

pub const DESCRIPTION: &str =
    "Start tracking a measurement the user should take regularly (e.g. \"watch my \
     temperature daily\", \"blood pressure every morning\"). Pass `metric` (required, \
     e.g. 'blood_pressure', 'glucose', 'body_temperature', 'body_weight'), and \
     optionally `label` (a friendly name), `schedule` (a machine cadence: a 5-field \
     cron expr like \"0 8 * * *\", or `anchor:<name>` such as `anchor:bedtime`), \
     `case_id` (the visit/episode that ordered it — omit for a personal watch), \
     `on_hand` (the user has the device), and `quick` (surface as a one-tap \
     shortcut). Readings themselves are logged with log_measurement. Returns the \
     minted `watch_id`.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "metric":   { "type": "string", "description": "Measurement type, e.g. 'blood_pressure'." },
            "label":    { "type": "string", "description": "Friendly name, optional." },
            "schedule": { "type": "string", "description": "Machine cadence: cron expr or 'anchor:<name>'." },
            "case_id":  { "type": "string", "description": "Ordering visit's case ULID; omit for personal." },
            "on_hand":  { "type": "boolean", "description": "User has the measuring device." },
            "quick":    { "type": "boolean", "description": "Show as a one-tap shortcut." },
            "started":  { "type": "string", "description": "ISO 8601; defaults to now." }
        },
        "required": ["metric"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let metric = require_string(input, "metric")?;
    let started_ms = crate::put::ts_from(input, "started");
    let watch_id = ohd_storage_core::ulid::to_crockford(&ohd_storage_core::ulid::mint(started_ms));

    let mut channels = vec![
        ch_text("watch_id", watch_id.clone()),
        ch_text("metric", metric),
    ];
    if let Some(c) = ch_opt_text("label", opt_string(input, "label")) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("schedule", opt_string(input, "schedule")) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("case_id", opt_string(input, "case_id")) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_bool("on_hand", input.get("on_hand").and_then(|v| v.as_bool())) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_bool("quick", input.get("quick").and_then(|v| v.as_bool())) {
        channels.push(c);
    }
    let mut out = commit(
        storage,
        "measurement.watch_started".to_string(),
        started_ms,
        None,
        channels,
        None,
    )?;
    if let Value::Object(map) = &mut out {
        map.insert("watch_id".to_string(), json!(watch_id));
    }
    Ok(out)
}
