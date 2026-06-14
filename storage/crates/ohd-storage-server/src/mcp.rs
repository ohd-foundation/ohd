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
//! [`ohd_auth::resolve_token`].
//!
//! Two token kinds reach here:
//!  - **SelfSession** — the storage owner acting on their own data.
//!    Runs unscoped through [`ohd_mcp_core::wire::handle_json_rpc`]
//!    with `scope = None` — full catalog, no per-call intersection.
//!  - **Grant** — a third party (CORD on cord.ohd.dev, a clinician's
//!    desktop client, etc.) that the owner gave a share to. Same wire
//!    layer, but `scope = Some(ShareScope::from_grant(...))` — the
//!    catalog is filtered (operator tools removed, read-only grants
//!    hide write tools) and every `tools/call` intersects its
//!    `EventFilter` with the grant's read rules. Mirrors what the
//!    phone-side share responder does over a relay tunnel; the SaaS
//!    path skips the inner-TLS hop because the storage is already
//!    public.
//!
//! Device tokens are rejected at the door — they're write-only on the
//! OHDC surface and have no business minting tool calls.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::{Json, Router};
use ohd_mcp_core::wire::{self, ServerInfo};
use ohd_mcp_core::ShareScope;
use ohd_storage_core::auth::{self as ohd_auth, TokenKind};
use ohd_storage_core::format::now_ms;
use ohd_storage_core::grants;
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

    // 2. Resolve the scope from the token kind. SelfSession is unscoped
    //    (full catalog); Grant tokens carry a [`ShareScope`] derived
    //    from the underlying grant row, refreshed per request so a
    //    revoke / suspend / expiry mid-session takes effect on the
    //    very next call. Device tokens are rejected — they're
    //    write-only on the OHDC surface and have no MCP surface
    //    semantics.
    let scope: Option<ShareScope> = match token.kind {
        TokenKind::SelfSession => None,
        TokenKind::Grant => {
            let grant_id = match token.grant_id {
                Some(g) => g,
                None => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "jsonrpc": "2.0",
                            "id": Value::Null,
                            "error": { "code": -32_603, "message": "grant token resolved without a grant id" },
                        })),
                    )
                        .into_response();
                }
            };
            match state
                .storage
                .with_conn(|conn| grants::read_grant(conn, grant_id))
            {
                Ok(grant) => Some(ShareScope::from_grant(&grant, now_ms())),
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({
                            "jsonrpc": "2.0",
                            "id": Value::Null,
                            "error": { "code": -32_603, "message": format!("scope resolution failed: {e}") },
                        })),
                    )
                        .into_response();
                }
            }
        }
        TokenKind::Device => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32_001, "message": "not permitted: device tokens cannot use the agent surface" },
                })),
            )
                .into_response();
        }
    };

    // 3. Wire dispatch. The shared dispatcher handles JSON-RPC
    //    envelopes, method routing, parse errors, and notifications
    //    (returning None for the latter). When `scope = Some(_)` the
    //    catalog is filtered and every tool call is scope-intersected.
    match wire::handle_json_rpc(&body, &state.storage, scope.as_ref(), SAAS_SERVER_INFO) {
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

