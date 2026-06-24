//! `open_case` — open a clinical case (episode).

use crate::grant_json::case_to_json;
use crate::put::opt_string;
use crate::{ToolError, ToolResult};
use ohd_storage_core::cases::{create_case, read_case, NewCase};
use ohd_storage_core::{ulid, Storage};
use serde_json::{json, Value};

pub const NAME: &str = "open_case";

pub const DESCRIPTION: &str =
    "Open a clinical case — a grouping for an episode of care (a sickness, a course \
     of treatment, a hospital stay). Pass `case_type` (e.g. \"clinic_visit\", \
     \"illness\", \"hospital_admission\"; default \"clinic_visit\") and an optional \
     `label`. Returns the case_ulid; pass it to record_prescription / \
     record_lab_result and get_case_timeline.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "case_type": { "type": "string", "description": "Episode kind.", "default": "clinic_visit" },
            "label":     { "type": "string", "description": "Human-readable label." }
        },
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let case_type = opt_string(input, "case_type").unwrap_or_else(|| "clinic_visit".to_string());
    let new = NewCase {
        case_type,
        case_label: opt_string(input, "label"),
        parent_case_ulid: None,
        predecessor_case_ulid: None,
        inactivity_close_after_h: None,
        initial_filters: vec![],
        opening_authority_grant_id: None,
    };
    let (rowid, case_ulid) = storage
        .with_conn_mut(|conn| create_case(conn, &new))
        .map_err(|e| ToolError::Internal(format!("create_case: {e}")))?;
    let case = storage
        .with_conn(|conn| read_case(conn, rowid))
        .map_err(|e| ToolError::Internal(format!("read_case: {e}")))?;
    Ok(json!({
        "ok": true,
        "case_ulid": ulid::to_crockford(&case_ulid),
        "case": case_to_json(&case),
    }))
}
