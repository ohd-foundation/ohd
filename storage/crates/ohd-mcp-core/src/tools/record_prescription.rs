//! `record_prescription` — record a prescription in a case + start its regimen.

use crate::put::{ch_opt_int, ch_opt_real, ch_opt_text, ch_text, commit, opt_string, require_string, ts_from};
use crate::ToolResult;
use ohd_storage_core::{ulid, Storage};
use serde_json::{json, Value};

pub const NAME: &str = "record_prescription";

pub const DESCRIPTION: &str =
    "Record a prescription issued at a visit and start its medication regimen in one \
     step. Requires `case_id` (from record_doctor_visit / open_case) and \
     `medication_name`; optional `dose_value`, `dose_unit`, `frequency`, \
     `duration_days`, `entered_by`. Writes a clinical.prescription event into the case \
     AND starts a medication.regimen_started so the drug shows up in \
     list_active_regimens and doses can attach to it. Returns the regimen_id.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "case_id":         { "type": "string", "description": "The visit's case ULID." },
            "medication_name": { "type": "string" },
            "dose_value":      { "type": "number" },
            "dose_unit":       { "type": "string" },
            "frequency":       { "type": "string", "description": "e.g. 'twice daily'." },
            "duration_days":   { "type": "integer" },
            "entered_by":      { "type": "string", "enum": ["self_from_memory", "self_from_document", "practitioner", "pharmacy"], "default": "self_from_memory" },
            "when":            { "type": "string", "description": "ISO 8601; defaults to now." }
        },
        "required": ["case_id", "medication_name"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let case_id = require_string(input, "case_id")?;
    let name = require_string(input, "medication_name")?;
    let when_ms = ts_from(input, "when");
    let dose_value = input.get("dose_value").and_then(|v| v.as_f64());
    let dose_unit = opt_string(input, "dose_unit");
    let frequency = opt_string(input, "frequency");
    let regimen_id = ulid::to_crockford(&ulid::mint(when_ms));

    // 1. The regimen — so the drug appears in list_active_regimens and
    //    doses can attach. Tagged with the originating case.
    let mut reg_ch = vec![
        ch_text("regimen_id", regimen_id.clone()),
        ch_text("name", name.clone()),
        ch_text("case_id", case_id.clone()),
    ];
    if let Some(c) = ch_opt_real("dose_value", dose_value) {
        reg_ch.push(c);
    }
    if let Some(c) = ch_opt_text("dose_unit", dose_unit.clone()) {
        reg_ch.push(c);
    }
    if let Some(c) = ch_opt_text("frequency", frequency.clone()) {
        reg_ch.push(c);
    }
    commit(storage, "medication.regimen_started".to_string(), when_ms, None, reg_ch, None)?;

    // 2. The prescription record in the case.
    let entered_by = opt_string(input, "entered_by").unwrap_or_else(|| "self_from_memory".to_string());
    let mut rx_ch = vec![
        ch_text("case_id", case_id.clone()),
        ch_text("medication_name", name),
        ch_text("regimen_id", regimen_id.clone()),
        ch_text("entered_by", entered_by),
    ];
    if let Some(c) = ch_opt_real("dose_value", dose_value) {
        rx_ch.push(c);
    }
    if let Some(c) = ch_opt_text("dose_unit", dose_unit) {
        rx_ch.push(c);
    }
    if let Some(c) = ch_opt_text("frequency", frequency) {
        rx_ch.push(c);
    }
    if let Some(c) = ch_opt_int("duration_days", input.get("duration_days").and_then(|v| v.as_i64())) {
        rx_ch.push(c);
    }
    let rx = commit(storage, "clinical.prescription".to_string(), when_ms, None, rx_ch, None)?;

    Ok(json!({
        "ok": true,
        "regimen_id": regimen_id,
        "case_id": case_id,
        "prescription": rx,
    }))
}
