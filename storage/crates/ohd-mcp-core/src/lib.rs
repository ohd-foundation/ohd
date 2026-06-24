//! OHD shared agent-tool catalog + dispatch.
//!
//! One Rust implementation per tool (`query_events`, `summarize`,
//! `log_food`, `create_grant`, …) wrapping the storage core. Both the
//! Android app (uniffi → [`dispatch`]) and the standalone MCP server
//! (axum → [`dispatch`]) sit on this. Adding a new tool means one new
//! file under `src/tools/` + one entry in [`catalog`] — every consumer
//! picks it up automatically.
//!
//! Plan reference:
//! `/home/jakub/contracts/personal/.claude-home/plans/deep-dancing-teacup.md`.

use ohd_storage_core::Storage;
use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

pub mod event_json;
pub mod grant_json;
pub mod put;
pub mod scope;
pub mod tools;
pub mod wire;

pub use scope::{ShareScope, ToolKind};

// ---------------------------------------------------------------------------
// Named string aliases — zero runtime cost, big readability win across the
// tool implementations. "Load-bearing strings" carry domain meaning; plain
// `String` loses it at call sites (`Map<String, String>` vs `Map<ToolName,
// ToolJson>`). Skip the alias only when the context is unambiguous.
// ---------------------------------------------------------------------------

/// Namespace half of `(namespace, name)` event-type pairs.
pub type Namespace = String;
/// Local-name half of `(namespace, name)` event-type pairs.
pub type TypeName = String;
/// Dotted event-type identifier (e.g. `"food.eaten"`).
pub type EventType = String;
/// Channel path within an event (e.g. `"systolic_mmhg"`).
pub type ChannelPath = String;
/// 26-char Crockford ULID.
pub type Ulid26 = String;
/// Tool name in the catalog (e.g. `"query_events"`).
pub type ToolName = String;
/// JSON encoded as a string — used at the uniffi boundary where serde
/// Values are awkward to marshal.
pub type JsonStr = String;

/// Errors a tool can produce. Each variant maps cleanly onto an MCP
/// error code + an Anthropic-API-style tool result with `is_error: true`.
#[derive(Debug, Error)]
pub enum ToolError {
    /// The named tool isn't registered in the catalog.
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    /// Input failed schema or business-rule validation.
    #[error("invalid input: {0}")]
    InvalidInput(String),
    /// Underlying storage failed.
    #[error("storage error: {0}")]
    Storage(#[from] ohd_storage_core::Error),
    /// The request is outside what the share's grant permits. Distinct
    /// from [`Self::InvalidInput`]: the agent must treat this as "not
    /// permitted", never "no data" or "bad request". Only ever produced
    /// on the scoped (share-responder) dispatch path.
    #[error("not permitted: {0}")]
    NotPermitted(String),
    /// Anything that doesn't fit the above.
    #[error("internal: {0}")]
    Internal(String),
}

pub type ToolResult<T> = Result<T, ToolError>;

/// Catalogue entry as both consumers see it. The `input_schema` field is
/// JSON Schema 2020-12 (compatible with Anthropic tool-use and MCP
/// `tools/list`).
// MCP 2025-03-26 wire shape is camelCase (`inputSchema`). The uniffi
// catalog consumers receive the same JSON via `catalog_json()` and
// don't care about the field name as long as it's stable; renaming
// at the serde layer keeps a single Rust struct serving both
// transports without a hand-converted MCP variant.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: ToolName,
    pub description: String,
    pub input_schema: Value,
}

/// Every tool the catalog ships. Order matters only for stable presentation.
///
/// This is the unscoped (owner) catalog. The share responder calls
/// [`catalog_scoped`] instead, which omits the tools a grant disallows.
pub fn catalog() -> Vec<Tool> {
    macro_rules! t {
        ($m:path) => {{
            use $m as inner;
            Tool {
                name: inner::NAME.to_string(),
                description: inner::DESCRIPTION.to_string(),
                input_schema: inner::input_schema(),
            }
        }};
    }
    vec![
        t!(tools::now),
        t!(tools::query_events),
        t!(tools::query_latest),
        t!(tools::describe_data),
        t!(tools::summarize),
        t!(tools::correlate),
        t!(tools::chart),
        t!(tools::get_food_log),
        t!(tools::get_medications_taken),
        t!(tools::log_symptom),
        t!(tools::log_food),
        t!(tools::log_medication),
        t!(tools::log_measurement),
        t!(tools::log_exercise),
        t!(tools::log_mood),
        t!(tools::log_sleep),
        t!(tools::log_free_event),
        t!(tools::list_grants),
        t!(tools::revoke_grant),
        t!(tools::list_pending),
        t!(tools::approve_pending),
        t!(tools::reject_pending),
        t!(tools::list_cases),
        t!(tools::get_case),
        t!(tools::force_close_case),
        t!(tools::create_grant),
        t!(tools::issue_retrospective_grant),
        t!(tools::audit_query),
        // Persistent facts + health profile.
        t!(tools::record_allergy),
        t!(tools::remove_allergy),
        t!(tools::list_allergies),
        t!(tools::record_condition),
        t!(tools::resolve_condition),
        t!(tools::list_conditions),
        t!(tools::set_blood_type),
        t!(tools::record_emergency_contact),
        t!(tools::remove_emergency_contact),
        t!(tools::list_emergency_contacts),
        t!(tools::get_health_profile),
        // Medication regimens.
        t!(tools::start_medication_regimen),
        t!(tools::discontinue_medication_regimen),
        t!(tools::list_active_regimens),
        // Measurement watches + treatment plan.
        t!(tools::start_measurement_watch),
        t!(tools::stop_measurement_watch),
        t!(tools::list_measurement_watches),
        t!(tools::get_treatment_plan),
        // Clinical cases + events.
        t!(tools::open_case),
        t!(tools::close_case),
        t!(tools::record_doctor_visit),
        t!(tools::record_prescription),
        t!(tools::record_lab_result),
        t!(tools::get_case_timeline),
        // Destructive owner-only escape hatch.
        t!(tools::delete_event),
    ]
}

/// Execute a tool by name. Returns errors instead of panicking so the
/// calling transport (uniffi vs MCP) can decide how to surface them.
pub fn dispatch(name: &str, input: &Value, storage: &Storage) -> ToolResult<Value> {
    match name {
        tools::now::NAME => tools::now::execute(input, storage),
        tools::query_events::NAME => tools::query_events::execute(input, storage),
        tools::query_latest::NAME => tools::query_latest::execute(input, storage),
        tools::describe_data::NAME => tools::describe_data::execute(input, storage),
        tools::summarize::NAME => tools::summarize::execute(input, storage),
        tools::correlate::NAME => tools::correlate::execute(input, storage),
        tools::chart::NAME => tools::chart::execute(input, storage),
        tools::get_food_log::NAME => tools::get_food_log::execute(input, storage),
        tools::get_medications_taken::NAME => tools::get_medications_taken::execute(input, storage),
        tools::log_symptom::NAME => tools::log_symptom::execute(input, storage),
        tools::log_food::NAME => tools::log_food::execute(input, storage),
        tools::log_medication::NAME => tools::log_medication::execute(input, storage),
        tools::log_measurement::NAME => tools::log_measurement::execute(input, storage),
        tools::log_exercise::NAME => tools::log_exercise::execute(input, storage),
        tools::log_mood::NAME => tools::log_mood::execute(input, storage),
        tools::log_sleep::NAME => tools::log_sleep::execute(input, storage),
        tools::log_free_event::NAME => tools::log_free_event::execute(input, storage),
        tools::list_grants::NAME => tools::list_grants::execute(input, storage),
        tools::revoke_grant::NAME => tools::revoke_grant::execute(input, storage),
        tools::list_pending::NAME => tools::list_pending::execute(input, storage),
        tools::approve_pending::NAME => tools::approve_pending::execute(input, storage),
        tools::reject_pending::NAME => tools::reject_pending::execute(input, storage),
        tools::list_cases::NAME => tools::list_cases::execute(input, storage),
        tools::get_case::NAME => tools::get_case::execute(input, storage),
        tools::force_close_case::NAME => tools::force_close_case::execute(input, storage),
        tools::create_grant::NAME => tools::create_grant::execute(input, storage),
        tools::issue_retrospective_grant::NAME => tools::issue_retrospective_grant::execute(input, storage),
        tools::audit_query::NAME => tools::audit_query::execute(input, storage),
        tools::record_allergy::NAME => tools::record_allergy::execute(input, storage),
        tools::remove_allergy::NAME => tools::remove_allergy::execute(input, storage),
        tools::list_allergies::NAME => tools::list_allergies::execute(input, storage),
        tools::record_condition::NAME => tools::record_condition::execute(input, storage),
        tools::resolve_condition::NAME => tools::resolve_condition::execute(input, storage),
        tools::list_conditions::NAME => tools::list_conditions::execute(input, storage),
        tools::set_blood_type::NAME => tools::set_blood_type::execute(input, storage),
        tools::record_emergency_contact::NAME => tools::record_emergency_contact::execute(input, storage),
        tools::remove_emergency_contact::NAME => tools::remove_emergency_contact::execute(input, storage),
        tools::list_emergency_contacts::NAME => tools::list_emergency_contacts::execute(input, storage),
        tools::get_health_profile::NAME => tools::get_health_profile::execute(input, storage),
        tools::start_medication_regimen::NAME => tools::start_medication_regimen::execute(input, storage),
        tools::discontinue_medication_regimen::NAME => tools::discontinue_medication_regimen::execute(input, storage),
        tools::list_active_regimens::NAME => tools::list_active_regimens::execute(input, storage),
        tools::start_measurement_watch::NAME => tools::start_measurement_watch::execute(input, storage),
        tools::stop_measurement_watch::NAME => tools::stop_measurement_watch::execute(input, storage),
        tools::list_measurement_watches::NAME => tools::list_measurement_watches::execute(input, storage),
        tools::get_treatment_plan::NAME => tools::get_treatment_plan::execute(input, storage),
        tools::open_case::NAME => tools::open_case::execute(input, storage),
        tools::close_case::NAME => tools::close_case::execute(input, storage),
        tools::record_doctor_visit::NAME => tools::record_doctor_visit::execute(input, storage),
        tools::record_prescription::NAME => tools::record_prescription::execute(input, storage),
        tools::record_lab_result::NAME => tools::record_lab_result::execute(input, storage),
        tools::get_case_timeline::NAME => tools::get_case_timeline::execute(input, storage),
        tools::delete_event::NAME => tools::delete_event::execute(input, storage),
        other => Err(ToolError::UnknownTool(other.to_string())),
    }
}

/// JSON-string convenience wrapper for the uniffi boundary where
/// `serde_json::Value` is awkward to marshal. On failure returns
/// `{"error": "..."}` stringified — Kotlin gets one shape regardless.
pub fn dispatch_json(name: &str, input_json: &str, storage: &Storage) -> JsonStr {
    let input: Value = match serde_json::from_str(input_json) {
        Ok(v) => v,
        Err(e) => return error_json(&format!("invalid JSON input: {e}")),
    };
    match dispatch(name, &input, storage) {
        Ok(v) => v.to_string(),
        Err(e) => error_json(&e.to_string()),
    }
}

/// Catalog as a JSON array — what uniffi consumers + MCP `tools/list` hand to clients.
pub fn catalog_json() -> JsonStr {
    serde_json::to_string(&catalog()).unwrap_or_else(|_| "[]".to_string())
}

fn error_json(message: &str) -> JsonStr {
    serde_json::json!({ "error": message }).to_string()
}

// ---------------------------------------------------------------------------
// Share-scoped surface — used by the phone-side share responder when serving
// a remote consumer (CORD, a clinician's device) over the relay tunnel.
//
// `scope: None` is exactly the owner path above — every scoped function with
// `None` delegates to its unscoped counterpart, so the local-CORD path is
// unchanged. `scope: Some(_)` constrains the catalog + every tool call to
// what the share's grant permits.
// ---------------------------------------------------------------------------

/// Catalog filtered to the tools a share's scope permits.
///
/// `None` returns the full owner catalog. `Some(scope)` omits operator
/// tools entirely and omits every write tool unless the grant carries
/// write rules. Read + utility tools are always listed (a denied scope
/// still lists them — calls then return [`ToolError::NotPermitted`] — so
/// the consumer sees a stable catalog rather than an empty one).
pub fn catalog_scoped(scope: Option<&ShareScope>) -> Vec<Tool> {
    let all = catalog();
    match scope {
        None => all,
        Some(scope) => all
            .into_iter()
            .filter(|t| scope.allows_tool_kind(scope::tool_kind(&t.name)))
            .collect(),
    }
}

/// JSON form of [`catalog_scoped`].
pub fn catalog_scoped_json(scope: Option<&ShareScope>) -> JsonStr {
    serde_json::to_string(&catalog_scoped(scope)).unwrap_or_else(|_| "[]".to_string())
}

/// Execute a tool, enforcing a share scope when one is supplied.
///
/// `scope = None` is identical to [`dispatch`]. `scope = Some(_)`:
///
/// - rejects operator tools and (for a read-only grant) write tools with
///   [`ToolError::NotPermitted`];
/// - rejects a write whose `event_type` the grant does not allow;
/// - for read tools, rejects an explicitly-named out-of-scope
///   `event_type`, clamps the requested time window to the grant's
///   window, and redacts out-of-scope event rows + channels from the
///   result;
/// - denies everything when the grant is suspended / revoked / expired.
pub fn dispatch_scoped(
    name: &str,
    input: &Value,
    storage: &Storage,
    scope: Option<&ShareScope>,
) -> ToolResult<Value> {
    let Some(scope) = scope else {
        return dispatch(name, input, storage);
    };

    let kind = scope::tool_kind(name);
    if !scope.allows_tool_kind(kind) {
        return Err(ToolError::NotPermitted(match kind {
            ToolKind::Operator => format!("tool {name} is owner-only and not available to a share"),
            ToolKind::Write => "this share is read-only; write tools are not permitted".to_string(),
            _ => format!("tool {name} is not available to this share"),
        }));
    }
    if let Some(reason) = scope.deny_reason() {
        return Err(ToolError::NotPermitted(reason.to_string()));
    }

    match kind {
        ToolKind::Utility => dispatch(name, input, storage),
        ToolKind::Operator => unreachable!("operator tools rejected above"),
        ToolKind::Write => {
            // Resolve the event type the call would write and check it
            // against the grant's write rules before touching storage.
            if let Some(et) = write_event_type(name, input) {
                if !scope.allows_write_type(&et) {
                    return Err(ToolError::NotPermitted(format!(
                        "this share does not permit writing {et} events"
                    )));
                }
            }
            dispatch(name, input, storage)
        }
        ToolKind::Read => {
            // Reject an explicitly-named out-of-scope event type up front
            // so the agent gets "not permitted", never a misleading empty
            // result for data it cannot see.
            for key in ["event_type", "event_type_a", "event_type_b"] {
                if let Some(et) = input.get(key).and_then(|v| v.as_str()) {
                    if !scope.allows_read_type(et) {
                        return Err(ToolError::NotPermitted(format!(
                            "this share does not permit reading {et} events"
                        )));
                    }
                }
            }
            // Clamp the requested time window to the grant's window.
            let scoped_input = clamp_read_input(input, scope);
            let mut out = dispatch(name, &scoped_input, storage)?;
            scope.redact_result(&mut out);
            Ok(out)
        }
    }
}

/// JSON-string wrapper for [`dispatch_scoped`] — the share responder's
/// uniffi / MCP entry point. Mirrors [`dispatch_json`].
pub fn dispatch_scoped_json(
    name: &str,
    input_json: &str,
    storage: &Storage,
    scope: Option<&ShareScope>,
) -> JsonStr {
    let input: Value = match serde_json::from_str(input_json) {
        Ok(v) => v,
        Err(e) => return error_json(&format!("invalid JSON input: {e}")),
    };
    match dispatch_scoped(name, &input, storage, scope) {
        Ok(v) => v.to_string(),
        Err(e) => error_json(&e.to_string()),
    }
}

/// Resolve the dotted `event_type` a `log_*` tool would write, so the
/// share scope can check it against the grant's write rules before
/// touching storage. `None` when the type cannot be determined from the
/// input — dispatch then proceeds and the storage layer rejects it.
fn write_event_type(name: &str, input: &Value) -> Option<String> {
    match name {
        "log_food" => Some("food.eaten".to_string()),
        "log_medication" => Some("medication.taken".to_string()),
        "log_exercise" => Some("activity.exercise_session".to_string()),
        "log_mood" => Some("wellness.mood".to_string()),
        "log_sleep" => Some("activity.sleep".to_string()),
        // Caller-chosen event type.
        "log_measurement" | "log_free_event" => input
            .get("event_type")
            .and_then(|v| v.as_str())
            .map(String::from),
        // log_symptom derives `symptom.<slug>` from the `symptom` field.
        "log_symptom" => input
            .get("symptom")
            .and_then(|v| v.as_str())
            .map(|s| format!("symptom.{}", tools::log_symptom::slugify(s))),
        // Persistent-fact + clinical write tools land on fixed types.
        "record_allergy" | "remove_allergy" => Some("profile.allergy".to_string()),
        "record_condition" | "resolve_condition" => Some("profile.condition".to_string()),
        "set_blood_type" => Some("profile.blood_type".to_string()),
        "record_emergency_contact" | "remove_emergency_contact" => {
            Some("profile.emergency_contact".to_string())
        }
        "start_medication_regimen" => Some("medication.regimen_started".to_string()),
        "discontinue_medication_regimen" => Some("medication.regimen_discontinued".to_string()),
        "start_measurement_watch" => Some("measurement.watch_started".to_string()),
        "stop_measurement_watch" => Some("measurement.watch_stopped".to_string()),
        // Clinical-record writes. record_prescription also starts a regimen,
        // but the primary write the grant authorizes is the clinical record;
        // a grant permitting clinical.prescription is the right gate.
        "record_doctor_visit" => Some("clinical.visit".to_string()),
        "record_prescription" => Some("clinical.prescription".to_string()),
        "record_lab_result" => Some("clinical.lab_result".to_string()),
        _ => None,
    }
}

/// Rewrite a read tool's `from_iso` / `to_iso` so the effective window is
/// the intersection of the requested window and the grant's window. An
/// empty intersection collapses the window to a zero-width range so the
/// query returns no rows (the redaction pass also enforces this).
fn clamp_read_input(input: &Value, scope: &ShareScope) -> Value {
    let mut out = input.clone();
    let Value::Object(map) = &mut out else {
        return out;
    };
    let req_from = map
        .get("from_iso")
        .and_then(|v| v.as_str())
        .and_then(event_json::parse_iso);
    let req_to = map
        .get("to_iso")
        .and_then(|v| v.as_str())
        .and_then(event_json::parse_iso);
    let bounds = scope.clamp_window(req_from, req_to);
    if bounds.empty {
        // Force an empty window: lower bound after upper bound.
        let pin = bounds.to_ms.or(bounds.from_ms).unwrap_or(0);
        map.insert("from_iso".into(), Value::from(event_json::ms_to_iso(pin + 1)));
        map.insert("to_iso".into(), Value::from(event_json::ms_to_iso(pin)));
        return out;
    }
    if let Some(f) = bounds.from_ms {
        map.insert("from_iso".into(), Value::from(event_json::ms_to_iso(f)));
    }
    if let Some(t) = bounds.to_ms {
        map.insert("to_iso".into(), Value::from(event_json::ms_to_iso(t)));
    }
    out
}
