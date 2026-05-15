//! Minimal MCP client over HTTP JSON-RPC 2.0. Talks to a data source's
//! tool surface — `tools/list` to discover, `tools/call` to invoke.
//!
//! Phase 2 targets a plain JSON request/response MCP HTTP endpoint. The
//! relay-tunnelled transport arrives in Phase 4; only the constructor's
//! `endpoint` changes — the JSON-RPC surface is identical.

use crate::types::{AgentError, ToolDef};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicI64, Ordering};

pub struct McpClient {
    http: reqwest::Client,
    endpoint: String,
    token: Option<String>,
    next_id: AtomicI64,
}

impl McpClient {
    pub fn new(endpoint: impl Into<String>, token: Option<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint: endpoint.into(),
            token,
            next_id: AtomicI64::new(1),
        }
    }

    async fn rpc(&self, method: &str, params: Value) -> Result<Value, AgentError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let req = json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params });
        let mut builder = self
            .http
            .post(&self.endpoint)
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .json(&req);
        if let Some(t) = &self.token {
            builder = builder.bearer_auth(t);
        }
        let resp = builder
            .send()
            .await
            .map_err(|e| AgentError::Mcp(format!("{method}: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AgentError::Mcp(format!("{method}: HTTP {status}: {body}")));
        }
        let body: Value = resp
            .json()
            .await
            .map_err(|e| AgentError::Mcp(format!("{method}: response not JSON: {e}")))?;
        if let Some(err) = body.get("error") {
            return Err(AgentError::Mcp(format!("{method}: {err}")));
        }
        Ok(body.get("result").cloned().unwrap_or(Value::Null))
    }

    /// Discover the source's tool catalog. Sends `initialize` first (best
    /// effort — a stateless server may ignore it) then `tools/list`.
    pub async fn list_tools(&self) -> Result<Vec<ToolDef>, AgentError> {
        let _ = self
            .rpc(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "cord-agent", "version": env!("CARGO_PKG_VERSION") },
                }),
            )
            .await;
        let result = self.rpc("tools/list", json!({})).await?;
        let tools = result
            .get("tools")
            .and_then(|t| t.as_array())
            .ok_or_else(|| AgentError::Mcp("tools/list result had no `tools` array".into()))?;
        Ok(tools
            .iter()
            .filter_map(|t| {
                Some(ToolDef {
                    name: t.get("name")?.as_str()?.to_string(),
                    description: t
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string(),
                    input_schema: t
                        .get("inputSchema")
                        .or_else(|| t.get("input_schema"))
                        .cloned()
                        .unwrap_or_else(|| json!({ "type": "object" })),
                })
            })
            .collect())
    }

    /// Invoke a tool. Returns `(text, is_error)` — a tool-level failure is
    /// surfaced as `is_error = true`, not a transport `Err`.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<(String, bool), AgentError> {
        let result = self
            .rpc("tools/call", json!({ "name": name, "arguments": arguments }))
            .await?;
        let is_error = result
            .get("isError")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let text = match result.get("content").and_then(|c| c.as_array()) {
            Some(blocks) => blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n"),
            None => result.to_string(),
        };
        Ok((text, is_error))
    }
}
