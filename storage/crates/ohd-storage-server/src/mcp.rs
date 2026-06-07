//! SaaS MCP surface — `POST /mcp` on `ohd-storage-server`.
//!
//! Mirrors what the phone-side share responder serves at the inner-TLS
//! end of a relay tunnel, but for the case where the user's storage
//! lives on the server itself (OHD Cloud or a single-tenant self-host).
//! `cord.ohd.dev` reaches the same RPCs as it would over a relay; the
//! `mcp.ohd.dev` DNS name is a Caddy alias for this route.
//!
//! Wire shape — JSON-RPC 2.0, MCP 2024-11-05:
//!  - `initialize`               → server capabilities handshake.
//!  - `notifications/initialized`/`initialized` → silent (HTTP 204).
//!  - `ping`                     → empty result.
//!  - `tools/list`               → owner catalog (`ohd_mcp_core::catalog`).
//!  - `tools/call`               → `ohd_mcp_core::dispatch_json`.
//!  - any other                  → JSON-RPC `-32601 method not found`.
//!
//! Authentication is the same `Authorization: Bearer …` bearer that
//! flows over the Connect-RPC surface — the OHD-issued session token.
//! No new credential class. The token is resolved via
//! [`ohd_auth::resolve_token`] and checked for the [`ListTools`] /
//! [`ExecuteTool`] ops before the wire dispatcher runs.
//!
//! Scope handling: this is the **owner** path — a bearer is the user
//! acting on their own storage. There is no [`ShareScope`] applied. The
//! grant-bound shape lives on the phone responder; if SaaS ever needs
//! to serve a third-party grant token over the same `/mcp` endpoint
//! we'll grow a second branch that resolves the grant's scope (the
//! shared wire layer is already scope-aware — see
//! [`ohd_mcp_core::wire::handle_json_rpc`]).

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use ohd_mcp_core::wire::{self, ServerInfo};
use ohd_storage_core::auth::{self as ohd_auth, OhdcOp};
use ohd_storage_core::Storage;
use serde_json::{json, Value};
use std::sync::Arc;

/// Identifies this transport in MCP `initialize` responses.
const SAAS_SERVER_INFO: ServerInfo = ServerInfo {
    name: "ohd-storage-server",
    version: env!("CARGO_PKG_VERSION"),
};

/// Build the sub-router that owns `/mcp`. Merged into the main axum
/// router alongside the OAuth routes — Connect-RPC stays the fallback
/// service so OHDC traffic on the same host is unaffected.
pub fn router(storage: Arc<Storage>) -> Router {
    let state = McpState { storage };
    Router::new().route("/mcp", post(handle_rpc)).with_state(state)
}

#[derive(Clone)]
struct McpState {
    storage: Arc<Storage>,
}

/// One MCP JSON-RPC request: auth, then [`wire::handle_json_rpc`].
async fn handle_rpc(
    State(state): State<McpState>,
    headers: HeaderMap,
    body: String,
) -> impl IntoResponse {
    // 1. Bearer auth. The connectrpc handlers do the same lookup; failure
    //    surfaces as HTTP 401 here rather than a JSON-RPC error — agents
    //    treat 401 as "I need a new token", not "the call failed".
    let bearer = match extract_bearer(&headers) {
        Some(b) => b,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32_000, "message": "missing bearer token" },
                })),
            )
                .into_response();
        }
    };
    let token = match state
        .storage
        .with_conn(|conn| ohd_auth::resolve_token(conn, bearer))
    {
        Ok(t) => t,
        Err(e) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32_000, "message": format!("auth: {e}") },
                })),
            )
                .into_response();
        }
    };

    // 2. Surface-level op check — the connectrpc surface gates ListTools
    //    and ExecuteTool separately; here we admit a token that can do
    //    either and let the wire dispatcher pick by JSON-RPC method.
    //    Both ops have the same kind-policy, so passing one through
    //    `check_kind_for_op` is sufficient as the door-keep test.
    if let Err(e) = ohd_auth::check_kind_for_op(&token, OhdcOp::ListTools) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": { "code": -32_001, "message": format!("not permitted: {e}") },
            })),
        )
            .into_response();
    }

    // 3. Wire dispatch. Owner path (scope=None); the shared dispatcher
    //    handles JSON-RPC envelopes, method routing, parse errors, and
    //    notifications (returning None for the latter).
    match wire::handle_json_rpc(&body, &state.storage, None, SAAS_SERVER_INFO) {
        Some(value) => Json(value).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

/// Pull the bearer credential out of an `Authorization: Bearer …` header.
fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

