//! OHD CORD agent — the server-side conversation engine.
//!
//! A [`ModelProvider`] runs the LLM rounds; an [`McpClient`] reaches the
//! data source's tools; [`Agent`] ties them into a bounded tool-use loop
//! that streams [`AgentEvent`]s. `cord-server` puts those on an SSE wire.

pub mod agent;
pub mod anthropic;
pub mod mcp;
pub mod provider;
pub mod types;

pub use agent::Agent;
pub use anthropic::AnthropicProvider;
pub use mcp::{McpClient, RelayTarget};
pub use provider::ModelProvider;
pub use types::{AgentError, AgentEvent, ContentBlock, Message, RoundOutput, ToolDef};
