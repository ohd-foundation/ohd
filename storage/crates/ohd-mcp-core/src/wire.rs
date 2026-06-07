//! MCP JSON-RPC 2.0 wire layer — one implementation, three transports.
//!
//! The phone-side share responder (relay tunnel + inner TLS), the SaaS
//! storage server's `/mcp` HTTPS route, and any standalone single-user
//! binary all need to parse the same JSON-RPC envelopes, dispatch into
//! the same five MCP methods, and emit the same response shapes. Before
//! this module existed each transport carried its own copy of that
//! routing logic. Now each transport is a thin wrapper that:
//!
//! 1. takes bytes off its specific wire (TCP-with-newlines for the
//!    relay-side responder, HTTP body for the cloud route, stdio for the
//!    desktop binary);
//! 2. supplies authentication context — owner-bearer for the SaaS path,
//!    grant-resolved [`ShareScope`] for the relay path, nothing for the
//!    single-user binary;
//! 3. calls [`handle_json_rpc`] for the actual protocol work;
//! 4. writes the returned JSON back on the same wire.
//!
//! Methods covered (MCP 2024-11-05):
//!
//! - `initialize`               → server capabilities handshake
//! - `notifications/initialized`/`initialized` → silent notification
//! - `ping`                     → empty result
//! - `tools/list`               → [`catalog`] / [`catalog_scoped`]
//! - `tools/call`               → [`dispatch_json`] / [`dispatch_scoped_json`]
//! - any other                  → `-32601 method not found`
//!
//! Scope handling collapses on the [`Option<&ShareScope>`] parameter:
//! `Some` selects the scoped catalog + dispatch and lets out-of-scope
//! calls surface as `isError: true` content; `None` selects the unscoped
//! owner-path catalog + dispatch. The phone, server, and binary stay
//! oblivious to the distinction — they pass through whatever scope they
//! resolved (or `None`) and that's it.

use crate::{catalog, catalog_scoped, dispatch_json, dispatch_scoped_json, ShareScope};
use ohd_storage_core::Storage;
use serde_json::{json, Value};

/// Identifies the wrapping transport in the `initialize` response. Every
/// transport supplies its own [`name`] / [`version`] so the same wire
/// function can speak for the relay responder, the cloud `/mcp` route,
/// or the standalone binary without each having to assemble its own
/// envelope.
///
/// The string lifetimes are `'static` because [`env!`] expands to a
/// `&'static str` and every existing caller picks the name as a
/// compile-time literal; a runtime-derived name would still convert
/// cleanly with `Box::leak` if a future transport needed one.
#[derive(Debug, Clone, Copy)]
pub struct ServerInfo {
    pub name: &'static str,
    pub version: &'static str,
}

/// MCP protocol version this wire layer implements. Bumped only on a
/// real MCP spec change — clients negotiate against it during
/// `initialize` (we currently ignore the client's preferred version
/// and answer with ours, which is what every reference MCP server
/// does for 2024-11-05).
pub const PROTOCOL_VERSION: &str = "2024-11-05";

// Standard JSON-RPC 2.0 error codes we surface from the wire layer.
// Tool-domain errors are NOT JSON-RPC errors — they ride inside
// `tools/call`'s result as `isError: true`, per MCP, so the agent can
// reason about them as failed tool calls rather than failed transport.

/// Malformed JSON.
const PARSE_ERROR: i32 = -32_700;
/// Method name we don't know.
const METHOD_NOT_FOUND: i32 = -32_601;
/// `params` missing required fields.
const INVALID_PARAMS: i32 = -32_602;

/// Handle one JSON-RPC envelope.
///
/// Returns `Some(value)` for a request that carries an `id` (the
/// JSON-RPC response to write back) and `None` for a notification (a
/// frame without `id`, which JSON-RPC says gets no reply at all).
///
/// `body` is the raw JSON envelope as bytes-then-text — no
/// newline-framing is assumed; the caller takes care of that. A parse
/// failure returns a `-32700` response under a `null` id, matching the
/// JSON-RPC spec.
///
/// `scope = None` is the owner path. `scope = Some(_)` is the grant path
/// — every tool call funnels through [`dispatch_scoped_json`] and
/// `tools/list` filters via [`catalog_scoped`].
pub fn handle_json_rpc(
    body: &str,
    storage: &Storage,
    scope: Option<&ShareScope>,
    server: ServerInfo,
) -> Option<Value> {
    let req: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return Some(rpc_error(Value::Null, PARSE_ERROR, &format!("parse error: {e}")));
        }
    };
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or_else(|| json!({}));

    let result: Result<Value, (i32, String)> = match method {
        "initialize" => Ok(json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": {
                "name": server.name,
                "version": server.version,
            },
        })),
        "notifications/initialized" | "initialized" => {
            // Notifications carry no id; clients expect no reply at all.
            return None;
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({
            "tools": match scope {
                Some(s) => catalog_scoped(Some(s)),
                None => catalog(),
            },
        })),
        "tools/call" => call_tool(&params, storage, scope),
        other => Err((METHOD_NOT_FOUND, format!("method not found: {other}"))),
    };

    let id = id?;
    Some(match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err((code, message)) => rpc_error(id, code, &message),
    })
}

/// `tools/call` handler shared by the scoped + unscoped paths. Wraps the
/// tool's JSON output in the MCP `content`-blocks envelope so an
/// out-of-scope call from a grant path surfaces as `isError: true`
/// rather than as a JSON-RPC transport error — the agent must read it
/// as "not permitted", never as "no data".
fn call_tool(
    params: &Value,
    storage: &Storage,
    scope: Option<&ShareScope>,
) -> Result<Value, (i32, String)> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or((INVALID_PARAMS, "params.name is required".to_string()))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let arg_str = arguments.to_string();

    let result_str = match scope {
        Some(s) => dispatch_scoped_json(name, &arg_str, storage, Some(s)),
        None => dispatch_json(name, &arg_str, storage),
    };

    // Tool-domain failures are encoded as `{"error": "..."}` inside the
    // dispatch result so they round-trip cleanly across the uniffi
    // boundary. We mirror that here as MCP `isError: true` so an agent
    // can treat the same outcome consistently regardless of transport.
    let parsed: Value = serde_json::from_str(&result_str).unwrap_or(Value::Null);
    let is_error = parsed.get("error").is_some();
    Ok(json!({
        "content": [{ "type": "text", "text": result_str }],
        "isError": is_error,
    }))
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ohd_storage_core::{Storage, StorageConfig};
    use tempfile::TempDir;

    fn fresh_storage() -> (Storage, TempDir) {
        let dir = TempDir::new().expect("tempdir");
        let storage = Storage::open(StorageConfig::new(dir.path().join("wire-test.ohd")))
            .expect("open storage");
        (storage, dir)
    }

    const TEST_SERVER: ServerInfo = ServerInfo {
        name: "ohd-mcp-wire-test",
        version: "0.0.0",
    };

    #[test]
    fn initialize_returns_capabilities() {
        let (storage, _dir) = fresh_storage();
        let req = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let resp = handle_json_rpc(req, &storage, None, TEST_SERVER).expect("response");
        let result = resp.get("result").expect("result");
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], TEST_SERVER.name);
    }

    #[test]
    fn notification_returns_none() {
        let (storage, _dir) = fresh_storage();
        let req = r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#;
        assert!(handle_json_rpc(req, &storage, None, TEST_SERVER).is_none());
    }

    #[test]
    fn unknown_method_is_minus_32601() {
        let (storage, _dir) = fresh_storage();
        let req = r#"{"jsonrpc":"2.0","id":7,"method":"does/not/exist"}"#;
        let resp = handle_json_rpc(req, &storage, None, TEST_SERVER).expect("response");
        assert_eq!(resp["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn parse_error_carries_null_id() {
        let (storage, _dir) = fresh_storage();
        let resp = handle_json_rpc("not json", &storage, None, TEST_SERVER).expect("response");
        assert_eq!(resp["error"]["code"], PARSE_ERROR);
        assert!(resp["id"].is_null());
    }

    #[test]
    fn tools_list_owner_path_is_unscoped() {
        let (storage, _dir) = fresh_storage();
        let req = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#;
        let resp = handle_json_rpc(req, &storage, None, TEST_SERVER).expect("response");
        let tools = resp["result"]["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), catalog().len());
    }
}
