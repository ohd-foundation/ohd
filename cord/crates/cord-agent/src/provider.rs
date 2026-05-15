//! The model-provider abstraction. One `round` = one model call: given
//! the conversation and tools, return the assistant's content blocks and
//! why it stopped. Tool execution + looping live in [`crate::agent`].

use crate::types::{AgentError, Message, RoundOutput, ToolDef};
use async_trait::async_trait;

#[async_trait]
pub trait ModelProvider: Send + Sync {
    async fn round(
        &self,
        model: &str,
        system: &str,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<RoundOutput, AgentError>;
}
