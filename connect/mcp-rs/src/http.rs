//! HTTP transport — single POST endpoint that consumes JSON-RPC and
//! returns JSON-RPC. Compatible with the simple "json-rpc over HTTP"
//! MCP transport (not full Streamable-HTTP SSE; that's a follow-up).

use crate::dispatch::dispatch;
use crate::jsonrpc::{Request, Response, PARSE_ERROR};
use axum::extract::State;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use ohd_storage_core::Storage;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub storage: Arc<Storage>,
}

pub fn build_router(storage: Arc<Storage>) -> Router {
    let state = AppState { storage };
    Router::new()
        .route("/healthz", get(healthz))
        .route("/mcp", post(handle_rpc))
        // Also accept POST at root so the canonical "configure
        // mcp.ohd.dev" copy-paste in client UIs works without a trailing path.
        .route("/", post(handle_rpc))
        .route("/docs", get(docs))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
}

async fn handle_rpc(
    State(state): State<AppState>,
    body: String,
) -> impl IntoResponse {
    // Try to parse as a single request first; clients may batch.
    if let Ok(req) = serde_json::from_str::<Request>(&body) {
        let resp = dispatch(&req, &state.storage);
        return Json(serde_json::to_value(resp).unwrap_or_default()).into_response();
    }
    if let Ok(reqs) = serde_json::from_str::<Vec<Request>>(&body) {
        let resps: Vec<Response> = reqs.iter().map(|r| dispatch(r, &state.storage)).collect();
        return Json(serde_json::to_value(resps).unwrap_or_default()).into_response();
    }
    let parse_err = Response::err(None, PARSE_ERROR, "could not parse JSON-RPC body");
    Json(serde_json::to_value(parse_err).unwrap_or_default()).into_response()
}

async fn healthz() -> &'static str {
    "ok"
}

const DOCS_HTML: &str = include_str!("docs.html");
async fn docs() -> axum::response::Html<&'static str> {
    axum::response::Html(DOCS_HTML)
}
