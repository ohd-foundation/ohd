//! Shared agent types: the conversation shape, tool definitions, and the
//! events the agent streams out to the caller.

use serde::Serialize;
use serde_json::Value;

/// A tool the agent may call — name, description, JSON-Schema for inputs.
/// Comes verbatim from the data source's MCP `tools/list`.
#[derive(Clone, Debug)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// One block of conversation content. Mirrors the Anthropic content-block
/// model, which every provider impl maps onto.
#[derive(Clone, Debug)]
pub enum ContentBlock {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
}

/// A single conversation turn. `role` is `"user"` or `"assistant"`.
#[derive(Clone, Debug)]
pub struct Message {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: vec![ContentBlock::Text(text.into())],
        }
    }

    pub fn assistant_text(text: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: vec![ContentBlock::Text(text.into())],
        }
    }
}

/// What one model round produced: the assistant's content blocks plus why
/// it stopped (`"tool_use"` means it wants tools run before continuing).
#[derive(Clone, Debug)]
pub struct RoundOutput {
    pub blocks: Vec<ContentBlock>,
    pub stop_reason: String,
}

/// An event streamed from the agent to the transport layer. `cord-server`
/// serializes these straight onto an SSE channel.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A chunk of assistant-visible text.
    Text { delta: String },
    /// A tool is about to run — for a "calling <name>" status line.
    Tool { name: String },
    /// The turn finished cleanly.
    Done,
    /// The turn failed.
    Error { message: String },
}

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("model provider: {0}")]
    Provider(String),
    #[error("data source (MCP): {0}")]
    Mcp(String),
}
