//! Direct-HTTP MCP transport: MCP JSON-RPC 2.0 over a plain HTTP
//! request/response endpoint.
//!
//! This is the `kind = "direct"` path — a CA-cert storage URL reachable
//! without a relay (cloud / self-host). The relay-tunnelled transport for
//! `kind = "relay"` sources lives in [`super::relay`]; the JSON-RPC
//! surface both speak is identical.

use crate::types::AgentError;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicI64, Ordering};

/// MCP JSON-RPC over a plain HTTP endpoint.
pub struct HttpTransport {
    http: reqwest::Client,
    endpoint: String,
    token: Option<String>,
    next_id: AtomicI64,
}

impl HttpTransport {
    pub fn new(endpoint: impl Into<String>, token: Option<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint: endpoint.into(),
            token,
            next_id: AtomicI64::new(1),
        }
    }

    /// Issue one JSON-RPC call and return its `result` (or an
    /// [`AgentError::Mcp`] carrying a transport or protocol failure).
    pub async fn rpc(&self, method: &str, params: Value) -> Result<Value, AgentError> {
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
}
