//! `get_health_profile` — the persistent-facts bundle.
//!
//! One call returns everything an agent (or an emergency consumer)
//! needs for context: blood type, active allergies, active conditions,
//! active medication regimens, and emergency contacts. Reads the same
//! projections the individual list tools use.

use crate::tools::profile_common::{active_facts, flatten_event, query_all};
use crate::tools::regimen_common::active_regimens;
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "get_health_profile";

pub const DESCRIPTION: &str =
    "Get the user's persistent health profile in one call: blood type, active \
     allergies, active conditions, active medication regimens, and emergency \
     contacts. This is the right tool to consult for clinical context before \
     reasoning about symptoms, drug interactions, or emergencies. `mode` \
     'emergency' focuses on the life-critical fields; 'general' (default) returns \
     everything.";

pub fn input_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "mode": { "type": "string", "enum": ["general", "emergency"], "default": "general" }
        },
        "additionalProperties": false
    })
}

pub fn execute(input: &Value, storage: &Storage) -> ToolResult<Value> {
    let emergency = input.get("mode").and_then(|v| v.as_str()) == Some("emergency");

    // Blood type is a singleton: the latest profile.blood_type event.
    // query_all returns newest-first (ts DESC, id DESC), so element 0 is
    // the most recent — random ULID tails make max-by-ulid unreliable.
    let blood_type = query_all(storage, "profile.blood_type")?
        .first()
        .map(flatten_event);

    let allergies = active_facts(storage, "profile.allergy", &["removed"])?;
    let conditions = active_facts(storage, "profile.condition", &["resolved"])?;
    let medications = active_regimens(storage)?;

    let mut out = json!({
        "blood_type": blood_type,
        "allergies": allergies,
        "conditions": conditions,
        "medications": medications,
    });

    // Emergency mode keeps the life-critical set front-and-centre and
    // adds contacts + advance directives; general mode adds contacts too
    // but is otherwise the same bundle.
    let contacts = active_facts(storage, "profile.emergency_contact", &["removed"])?;
    out["emergency_contacts"] = json!(contacts);
    if emergency {
        let directives = active_facts(storage, "profile.advance_directive", &["removed"])?;
        out["advance_directives"] = json!(directives);
        out["mode"] = json!("emergency");
    } else {
        out["mode"] = json!("general");
    }
    Ok(out)
}
