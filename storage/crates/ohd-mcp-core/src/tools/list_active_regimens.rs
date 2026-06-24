//! `list_active_regimens` — the medications the user is currently on.

use crate::tools::regimen_common::active_regimens;
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "list_active_regimens";

pub const DESCRIPTION: &str =
    "List the medication regimens the user is currently on (started and not yet \
     discontinued). Each carries regimen_id, name, dose_value, dose_unit, frequency, \
     and the prescribing case_id when known. Use the regimen_id with log_medication \
     to attach doses.";

pub fn input_schema() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

pub fn execute(_input: &Value, storage: &Storage) -> ToolResult<Value> {
    let regimens = active_regimens(storage)?;
    Ok(json!({ "count": regimens.len(), "regimens": regimens }))
}
