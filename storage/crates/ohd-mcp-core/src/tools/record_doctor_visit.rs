//! `record_doctor_visit` — open a case for a visit + record the visit event.

use crate::grant_json::case_to_json;
use crate::put::{ch_opt_text, ch_text, commit, opt_string, require_string, ts_from};
use crate::tools::clinical_common::attach_case_filter;
use crate::{ToolError, ToolResult};
use ohd_storage_core::cases::{create_case, read_case, NewCase};
use ohd_storage_core::{ulid, Storage};
use serde_json::{json, Value};

pub const NAME: &str = "record_doctor_visit";

pub const DESCRIPTION: &str =
    "Record a doctor / clinic visit. Opens a clinical case for the episode and writes \
     a clinical.visit event into it. Pass `practitioner_name` (required) and optionally \
     `specialty`, `facility`, `reason`, and `when` (ISO 8601, default now). The user is \
     usually the source (they're entering it from memory or a report); pass \
     `entered_by` if known (self_from_memory | self_from_document | practitioner | \
     pharmacy). Returns the case_ulid — pass it to record_prescription / \
     record_lab_result to attach more to the same visit.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "practitioner_name": { "type": "string", "description": "Who the user saw." },
            "specialty":         { "type": "string" },
            "facility":          { "type": "string" },
            "reason":            { "type": "string", "description": "Why the visit happened." },
            "when":              { "type": "string", "description": "ISO 8601; defaults to now." },
            "entered_by":        { "type": "string", "enum": ["self_from_memory", "self_from_document", "practitioner", "pharmacy"], "default": "self_from_memory" },
            "case_ulid":         { "type": "string", "description": "Attach to an existing case instead of opening a new one." }
        },
        "required": ["practitioner_name"],
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let practitioner = require_string(input, "practitioner_name")?;
    let when_ms = ts_from(input, "when");

    // Reuse an existing case if supplied, else open a fresh one labelled
    // after the practitioner.
    let case_ulid = match opt_string(input, "case_ulid") {
        Some(c) => c,
        None => {
            let label = opt_string(input, "reason")
                .map(|r| format!("{practitioner} — {r}"))
                .unwrap_or_else(|| format!("Visit: {practitioner}"));
            let new = NewCase {
                case_type: "clinic_visit".to_string(),
                case_label: Some(label),
                parent_case_ulid: None,
                predecessor_case_ulid: None,
                inactivity_close_after_h: None,
                initial_filters: vec![],
                opening_authority_grant_id: None,
            };
            let (_, u) = storage
                .with_conn_mut(|conn| create_case(conn, &new))
                .map_err(|e| ToolError::Internal(format!("create_case: {e}")))?;
            ulid::to_crockford(&u)
        }
    };

    let entered_by = opt_string(input, "entered_by").unwrap_or_else(|| "self_from_memory".to_string());
    let mut channels = vec![
        ch_text("case_id", case_ulid.clone()),
        ch_text("practitioner_name", practitioner),
        ch_text("entered_by", entered_by),
    ];
    for key in ["specialty", "facility", "reason"] {
        if let Some(c) = ch_opt_text(key, opt_string(input, key)) {
            channels.push(c);
        }
    }
    let visit = commit(storage, "clinical.visit".to_string(), when_ms, None, channels, None)?;

    // Make the case scope include its members (for sharing). Best-effort:
    // the visit event is already tagged with case_id for direct queries.
    let _ = attach_case_filter(storage, &case_ulid);

    let rowid = crate::tools::clinical_common::case_rowid(storage, &case_ulid)?;
    let case = storage
        .with_conn(|conn| read_case(conn, rowid))
        .map_err(|e| ToolError::Internal(format!("read_case: {e}")))?;
    Ok(json!({
        "ok": true,
        "case_ulid": case_ulid,
        "visit": visit,
        "case": case_to_json(&case),
    }))
}
