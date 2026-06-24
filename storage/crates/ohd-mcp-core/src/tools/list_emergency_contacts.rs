//! `list_emergency_contacts` — the user's current emergency contacts.

use crate::tools::profile_common::active_facts;
use crate::ToolResult;
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const NAME: &str = "list_emergency_contacts";

pub const DESCRIPTION: &str =
    "List the user's emergency contacts (active only). Each entry carries name, \
     phone, relation, and a stable fact_id.";

pub fn input_schema() -> Value {
    json!({ "type": "object", "properties": {}, "additionalProperties": false })
}

pub fn execute(_input: &Value, storage: &Storage) -> ToolResult<Value> {
    let contacts = active_facts(storage, "profile.emergency_contact", &["removed"])?;
    Ok(json!({ "count": contacts.len(), "contacts": contacts }))
}
