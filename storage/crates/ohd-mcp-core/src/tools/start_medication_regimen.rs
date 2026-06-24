//! `start_medication_regimen` — begin a prescribed/taken course of a drug.
//!
//! Mints a `regimen_id` and writes a `medication.regimen_started` event.
//! Subsequent `log_medication` doses link to it by `regimen_id`. End it
//! with `discontinue_medication_regimen`.

use crate::put::{
    ch_opt_bool, ch_opt_real, ch_opt_text, ch_text, commit, opt_string, require_string,
};
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "start_medication_regimen";

pub const DESCRIPTION: &str =
    "Start a medication regimen (an ongoing course the user is on). Pass `name` \
     (required), and optionally `dose_value` + `dose_unit`, `frequency` (free text, \
     e.g. \"twice daily\"), `schedule` (a machine cadence: a 5-field cron expr like \
     \"0 */8 * * *\", or `anchor:<name>` such as `anchor:lunch`/`anchor:bedtime`/\
     `anchor:as_needed`), `case_id` (the visit it was prescribed at — omit for a \
     personal/self-started med), `on_hand` (the user physically has it), `quick` \
     (surface as a one-tap shortcut), and `rx_concept_id` (a drug-registry code, \
     reserved). Returns the minted `regimen_id` — pass it to log_medication so \
     doses attach to this regimen.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name":          { "type": "string", "description": "Medication name." },
            "dose_value":    { "type": "number" },
            "dose_unit":     { "type": "string" },
            "frequency":     { "type": "string", "description": "Free text, e.g. 'twice daily'." },
            "schedule":      { "type": "string", "description": "Machine cadence: cron expr or 'anchor:<name>'." },
            "case_id":       { "type": "string", "description": "Prescribing visit's case ULID; omit for personal." },
            "on_hand":       { "type": "boolean", "description": "User physically has the medication." },
            "quick":         { "type": "boolean", "description": "Show as a one-tap shortcut." },
            "rx_concept_id": { "type": "string", "description": "Drug-registry id (reserved), optional." },
            "started":       { "type": "string", "description": "ISO 8601; defaults to now." }
        },
        "required": ["name"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let name = require_string(input, "name")?;
    let started_ms = crate::put::ts_from(input, "started");
    // A fresh ULID is the regimen's stable identity (doses + the eventual
    // discontinue event reference it).
    let regimen_id = ohd_storage_core::ulid::to_crockford(&ohd_storage_core::ulid::mint(started_ms));

    let mut channels = vec![
        ch_text("regimen_id", regimen_id.clone()),
        ch_text("name", name),
    ];
    if let Some(c) = ch_opt_real("dose_value", input.get("dose_value").and_then(|v| v.as_f64())) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("dose_unit", opt_string(input, "dose_unit")) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("frequency", opt_string(input, "frequency")) {
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
    if let Some(c) = ch_opt_text("rx_concept_id", opt_string(input, "rx_concept_id")) {
        channels.push(c);
    }
    let mut out = commit(
        storage,
        "medication.regimen_started".to_string(),
        started_ms,
        None,
        channels,
        None,
    )?;
    // Surface the regimen_id so the caller can attach doses.
    if let Value::Object(map) = &mut out {
        map.insert("regimen_id".to_string(), json!(regimen_id));
    }
    Ok(out)
}
