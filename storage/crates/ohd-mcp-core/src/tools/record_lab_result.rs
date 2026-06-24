//! `record_lab_result` — record a lab test result in a case.

use crate::put::{ch_opt_real, ch_opt_text, ch_text, commit, opt_string, require_string, ts_from};
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "record_lab_result";

pub const DESCRIPTION: &str =
    "Record a lab test result. Requires `case_id` (from record_doctor_visit / \
     open_case) and `test_name`. Pass a numeric `value` + `unit`, or a `value_text` \
     for non-numeric results, plus optional `reference_range` and `entered_by`. \
     Writes a clinical.lab_result event into the case.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "case_id":         { "type": "string", "description": "The visit's case ULID." },
            "test_name":       { "type": "string" },
            "value":           { "type": "number", "description": "Numeric result." },
            "value_text":      { "type": "string", "description": "Non-numeric result (e.g. 'positive')." },
            "unit":            { "type": "string" },
            "reference_range": { "type": "string", "description": "e.g. '3.5-5.5'." },
            "entered_by":      { "type": "string", "enum": ["self_from_memory", "self_from_document", "practitioner", "pharmacy"], "default": "self_from_memory" },
            "when":            { "type": "string", "description": "ISO 8601; defaults to now." }
        },
        "required": ["case_id", "test_name"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let case_id = require_string(input, "case_id")?;
    let test_name = require_string(input, "test_name")?;
    let entered_by = opt_string(input, "entered_by").unwrap_or_else(|| "self_from_memory".to_string());
    let mut channels = vec![
        ch_text("case_id", case_id.clone()),
        ch_text("test_name", test_name),
        ch_text("entered_by", entered_by),
    ];
    if let Some(c) = ch_opt_real("value", input.get("value").and_then(|v| v.as_f64())) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("value_text", opt_string(input, "value_text")) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("unit", opt_string(input, "unit")) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("reference_range", opt_string(input, "reference_range")) {
        channels.push(c);
    }
    let out = commit(
        storage,
        "clinical.lab_result".to_string(),
        ts_from(input, "when"),
        None,
        channels,
        None,
    )?;
    Ok(json!({ "ok": true, "case_id": case_id, "lab_result": out }))
}
