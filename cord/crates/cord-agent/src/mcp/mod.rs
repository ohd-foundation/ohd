//! MCP client for a CORD data source.
//!
//! [`McpClient`] is **transport-agnostic**: it speaks MCP JSON-RPC 2.0
//! (`initialize` / `tools/list` / `tools/call`) and is parameterised over
//! how those bytes reach the storage:
//!
//! - [`Transport::Http`] — `kind = "direct"` sources: plain HTTP
//!   request/response to a CA-cert storage URL ([`http::HttpTransport`]).
//! - [`Transport::Relay`] — `kind = "relay"` sources: MCP tunnelled
//!   through OHD Relay over a pinned inner-TLS 1.3 session
//!   ([`relay::RelaySession`]). This is the Phase 4e data plane.
//!
//! `cord-server` picks the transport from the source's `kind` and hands
//! the resulting `McpClient` to the [`crate::Agent`]; the agent neither
//! knows nor cares which path is in use.

pub mod http;
pub mod relay;

use crate::types::{AgentError, ToolDef};
use serde_json::{json, Value};

pub use relay::RelayTarget;

/// How an [`McpClient`] reaches its data source.
enum Transport {
    /// Direct HTTP to a CA-cert storage URL.
    Http(http::HttpTransport),
    /// Relay-tunnelled, pinned inner-TLS. Holds the dial parameters; a
    /// fresh tunnel session is opened per call (`CORD reconnects on
    /// demand for each chat`, per the data-link spec).
    Relay(RelayTarget),
}

/// An MCP client bound to one data source.
pub struct McpClient {
    transport: Transport,
}

impl McpClient {
    /// A direct-HTTP client for a `kind = "direct"` source.
    pub fn new(endpoint: impl Into<String>, token: Option<String>) -> Self {
        Self {
            transport: Transport::Http(http::HttpTransport::new(endpoint, token)),
        }
    }

    /// A relay-tunnelled client for a `kind = "relay"` source. `target`
    /// carries the rendezvous id, relay host, unsealed grant token, and
    /// the storage-identity cert pin from the share link.
    pub fn relay(target: RelayTarget) -> Self {
        Self {
            transport: Transport::Relay(target),
        }
    }

    /// Probe reachability: open the transport far enough to complete the
    /// MCP `initialize` exchange, then drop it. Used by
    /// `POST /v1/sources/:id/refresh`.
    ///
    /// For a relay source this opens the full tunnel — relay attach,
    /// pinned inner-TLS handshake, MCP `initialize` — which is exactly
    /// the reachability the connect flow needs to verify.
    pub async fn probe(&self) -> Result<(), AgentError> {
        match &self.transport {
            Transport::Http(http) => {
                http.rpc(
                    "initialize",
                    json!({
                        "protocolVersion": "2024-11-05",
                        "capabilities": {},
                        "clientInfo": {
                            "name": "cord-agent",
                            "version": env!("CARGO_PKG_VERSION"),
                        },
                    }),
                )
                .await
                .map(|_| ())
            }
            Transport::Relay(target) => {
                // `connect` runs relay attach + pinned TLS + `initialize`.
                relay::RelaySession::connect(target).await.map(|_| ())
            }
        }
    }

    /// Discover the source's tool catalog. Sends `initialize` first (best
    /// effort for HTTP — a stateless server may ignore it; mandatory for
    /// the relay session) then `tools/list`.
    pub async fn list_tools(&self) -> Result<Vec<ToolDef>, AgentError> {
        let result = match &self.transport {
            Transport::Http(http) => {
                let _ = http
                    .rpc(
                        "initialize",
                        json!({
                            "protocolVersion": "2024-11-05",
                            "capabilities": {},
                            "clientInfo": {
                                "name": "cord-agent",
                                "version": env!("CARGO_PKG_VERSION"),
                            },
                        }),
                    )
                    .await;
                http.rpc("tools/list", json!({})).await?
            }
            Transport::Relay(target) => {
                // `connect` already performed `initialize`.
                let mut session = relay::RelaySession::connect(target).await?;
                session.rpc("tools/list", json!({})).await?
            }
        };
        parse_tools(result)
    }

    /// Invoke a tool. Returns `(text, is_error)` — a tool-level failure is
    /// surfaced as `is_error = true`, not a transport `Err`.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: Value,
    ) -> Result<(String, bool), AgentError> {
        let params = json!({ "name": name, "arguments": arguments });
        let result = match &self.transport {
            Transport::Http(http) => http.rpc("tools/call", params).await?,
            Transport::Relay(target) => {
                let mut session = relay::RelaySession::connect(target).await?;
                session.rpc("tools/call", params).await?
            }
        };
        Ok(parse_tool_result(result))
    }
}

/// Map a `tools/list` JSON `result` into [`ToolDef`]s.
fn parse_tools(result: Value) -> Result<Vec<ToolDef>, AgentError> {
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

/// Flatten a `tools/call` JSON `result` into `(text, is_error)`.
fn parse_tool_result(result: Value) -> (String, bool) {
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
    (text, is_error)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tools_extracts_catalog() {
        let result = json!({
            "tools": [
                { "name": "now", "description": "clock", "inputSchema": { "type": "object" } },
                { "name": "describe_data", "input_schema": { "type": "object" } },
            ]
        });
        let tools = parse_tools(result).unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "now");
        assert_eq!(tools[0].description, "clock");
        // `input_schema` (snake_case) accepted as a fallback.
        assert_eq!(tools[1].name, "describe_data");
        assert_eq!(tools[1].description, "");
    }

    #[test]
    fn parse_tools_missing_array_errors() {
        assert!(parse_tools(json!({})).is_err());
    }

    #[test]
    fn parse_tool_result_joins_text_blocks() {
        let result = json!({
            "content": [ { "type": "text", "text": "a" }, { "type": "text", "text": "b" } ]
        });
        let (text, is_error) = parse_tool_result(result);
        assert_eq!(text, "a\nb");
        assert!(!is_error);
    }

    #[test]
    fn parse_tool_result_flags_error() {
        let result = json!({
            "isError": true,
            "content": [ { "type": "text", "text": "permission denied" } ]
        });
        let (text, is_error) = parse_tool_result(result);
        assert_eq!(text, "permission denied");
        assert!(is_error);
    }

    #[test]
    fn relay_constructor_selects_relay_transport() {
        let client = McpClient::relay(RelayTarget {
            relay_host: "https://relay.ohd.dev".into(),
            rendezvous_id: "RV1".into(),
            pin: "AAAA".into(),
            token: "ohdg_x".into(),
        });
        assert!(matches!(client.transport, Transport::Relay(_)));
    }

    #[test]
    fn http_constructor_selects_http_transport() {
        let client = McpClient::new("https://storage.example/mcp", Some("t".into()));
        assert!(matches!(client.transport, Transport::Http(_)));
    }
}
