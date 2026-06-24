//! `set_blood_type` — record the user's blood type (singleton; latest wins).

use crate::put::{ch_opt_text, ch_text, commit, opt_string, require_string};
use crate::{ToolError, ToolResult};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "set_blood_type";

pub const DESCRIPTION: &str =
    "Set the user's blood type. Pass `group` (A | B | AB | O) and `rh` \
     (positive | negative | unknown), optionally `detail` for extended phenotype \
     (e.g. \"Kell-negative\"). This is a singleton — the latest value wins; there's \
     no list, read it via get_health_profile.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "group":  { "type": "string", "enum": ["A", "B", "AB", "O"] },
            "rh":     { "type": "string", "enum": ["positive", "negative", "unknown"], "default": "unknown" },
            "detail": { "type": "string", "description": "Extended phenotype, optional." }
        },
        "required": ["group"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let group = require_string(input, "group")?;
    if !["A", "B", "AB", "O"].contains(&group.as_str()) {
        return Err(ToolError::InvalidInput("group must be one of A, B, AB, O".into()));
    }
    let rh = opt_string(input, "rh").unwrap_or_else(|| "unknown".to_string());
    let mut channels = vec![ch_text("group", group), ch_text("rh", rh)];
    if let Some(c) = ch_opt_text("detail", opt_string(input, "detail")) {
        channels.push(c);
    }
    commit(
        storage,
        "profile.blood_type".to_string(),
        crate::event_json::now_ms(),
        None,
        channels,
        None,
    )
}
