//! `get_treatment_plan` — the medications + measurement watches for a case,
//! or for the implicit global "life" case (no case_id) when none is given.

use crate::tools::regimen_common::active_regimens;
use crate::tools::watch_common::active_watches;
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "get_treatment_plan";

pub const DESCRIPTION: &str =
    "Get the treatment plan — the active medication regimens and measurement \
     watches — for a case. Pass `case_id` for a specific clinical episode, or omit \
     it to get the user's personal/standing items (the implicit global \"life\" \
     case: everything not tied to a specific case). Returns { case_id, medications, \
     watches }.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "case_id": { "type": "string", "description": "Episode ULID; omit for personal/standing items." }
        },
        "additionalProperties": false
    })
}

/// True when `item`'s `case_id` channel matches the requested filter:
/// a specific case → equal; no filter → the item also has no case_id.
fn in_scope(item: &Value, want: Option<&str>) -> bool {
    let item_case = item.get("case_id").and_then(Value::as_str).filter(|s| !s.is_empty());
    match want {
        Some(c) => item_case == Some(c),
        None => item_case.is_none(),
    }
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let want = input.get("case_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty());

    let medications: Vec<Value> = active_regimens(storage)?
        .into_iter()
        .filter(|r| in_scope(r, want))
        .collect();
    let watches: Vec<Value> = active_watches(storage)?
        .into_iter()
        .filter(|w| in_scope(w, want))
        .collect();

    Ok(json!({
        "case_id": want,
        "medications": medications,
        "watches": watches,
    }))
}
