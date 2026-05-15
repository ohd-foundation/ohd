//! Map MCP JSON-RPC methods → `ohd_mcp_core` calls.
//!
//! v1 surface (per MCP 2024-11-05):
//!  - `initialize`               → handshake
//!  - `notifications/initialized` → silent ack
//!  - `tools/list`               → catalog
//!  - `tools/call`               → execute one tool
//!
//! Future: `prompts/list` / `prompts/get` once `ohd-mcp-core` grows a
//! `skills/` module (see `connect/android/missing_features.md`).

use crate::jsonrpc::{Request, Response, INTERNAL_ERROR, INVALID_PARAMS, METHOD_NOT_FOUND};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

pub const PROTOCOL_VERSION: &str = "2024-11-05";

pub fn dispatch(req: &Request, storage: &Storage) -> Response {
    match req.method.as_str() {
        "initialize" => Response::ok(req.id.clone(), json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "tools": { "listChanged": false },
            },
            "serverInfo": {
                "name": "ohd-mcp-rs",
                "version": env!("CARGO_PKG_VERSION"),
            },
        })),
        "notifications/initialized" | "initialized" => {
            // Notifications carry no `id` and expect no response, but
            // returning a benign one is harmless for clients that
            // mistakenly treat them as requests.
            Response::ok(req.id.clone(), json!({}))
        }
        "tools/list" => Response::ok(req.id.clone(), json!({
            "tools": ohd_mcp_core::catalog(),
        })),
        "tools/call" => call_tool(req, storage),
        "ping" => Response::ok(req.id.clone(), json!({})),
        other => Response::err(
            req.id.clone(),
            METHOD_NOT_FOUND,
            format!("method not found: {other}"),
        ),
    }
}

fn call_tool(req: &Request, storage: &Storage) -> Response {
    let params = match req.params.as_ref() {
        Some(p) => p,
        None => return Response::err(req.id.clone(), INVALID_PARAMS, "missing params"),
    };
    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return Response::err(req.id.clone(), INVALID_PARAMS, "params.name is required"),
    };
    let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    let arg_str = arguments.to_string();
    let result_str = ohd_mcp_core::dispatch_json(name, &arg_str, storage);

    // The MCP `tools/call` contract: result wraps content blocks. We
    // encode tool output as a single text block carrying the JSON.
    let parsed: Value = serde_json::from_str(&result_str).unwrap_or(Value::Null);
    let is_error = parsed.get("error").is_some();
    Response::ok(req.id.clone(), json!({
        "content": [{ "type": "text", "text": result_str }],
        "isError": is_error,
    }))
}

#[allow(dead_code)]
fn internal_error(req: &Request, msg: impl Into<String>) -> Response {
    Response::err(req.id.clone(), INTERNAL_ERROR, msg)
}
