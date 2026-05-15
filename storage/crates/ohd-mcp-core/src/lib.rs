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
pub mod tools;

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
    /// Anything that doesn't fit the above.
    #[error("internal: {0}")]
    Internal(String),
}

pub type ToolResult<T> = Result<T, ToolError>;

/// Catalogue entry as both consumers see it. The `input_schema` field is
/// JSON Schema 2020-12 (compatible with Anthropic tool-use and MCP
/// `tools/list`).
#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    pub name: ToolName,
    pub description: String,
    pub input_schema: Value,
}

/// Every tool the catalog ships. Order matters only for stable presentation.
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
