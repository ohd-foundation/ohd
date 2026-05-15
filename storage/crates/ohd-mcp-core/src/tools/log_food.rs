//! `log_food` — record a food/drink the user consumed.
//!
//! Writes a single `food.eaten` event with the name, grams (if the
//! quantity parses), barcode, and free-text notes. Per-nutrient
//! `intake.*` child events are NOT emitted by this tool — those come
//! from the FoodDetail screen when the user scans an OFF product. The
//! agent typically logs unstructured ("ate an apple") and the user can
//! re-resolve via barcode later.

use crate::put::{ch_opt_real, ch_opt_text, ch_text, commit, opt_string, require_string, ts_from};
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "log_food";

pub const DESCRIPTION: &str =
    "Log a food or drink the user consumed (event_type = `food.eaten`). Pass \
     `description` (required) and optionally `grams` for a known amount, \
     `barcode` for a future OFF lookup, ISO 8601 `started` / `ended`, and \
     `notes`. The per-nutrient `intake.*` child events are populated only \
     when the user scans a barcode in-app; this tool only writes the parent.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "description": { "type": "string", "description": "What was eaten or drunk." },
            "grams":       { "type": "number", "minimum": 0 },
            "barcode":     { "type": "string", "description": "EAN-13 / UPC barcode (digits only)." },
            "started":     { "type": "string", "description": "ISO 8601; defaults to now." },
            "ended":       { "type": "string" },
            "notes":       { "type": "string" }
        },
        "required": ["description"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let name = require_string(input, "description")?;
    let started_ms = ts_from(input, "started");
    let ended_ms = input.get("ended").and_then(|v| v.as_str()).and_then(crate::event_json::parse_iso);
    let duration_ms = ended_ms.map(|e| (e - started_ms).max(0));

    let mut channels = vec![ch_text("name", name)];
    if let Some(c) = ch_opt_real("grams", input.get("grams").and_then(|v| v.as_f64())) {
        channels.push(c);
    }
    if let Some(c) = ch_opt_text("barcode", opt_string(input, "barcode")) {
        channels.push(c);
    }
    commit(
        storage,
        "food.eaten".to_string(),
        started_ms,
        duration_ms,
        channels,
        opt_string(input, "notes"),
    )
}
