//! The model-provider abstraction. One `round` = one model call: given
//! the conversation and tools, return the assistant's content blocks and
//! why it stopped. Tool execution + looping live in [`crate::agent`].
//!
//! A round streams assistant text as it arrives: the provider emits
//! `AgentEvent::Text` deltas on the supplied channel token-by-token, then
//! returns the fully assembled [`RoundOutput`] when the round completes.

use crate::types::{AgentError, AgentEvent, Message, RoundOutput, ToolDef};
use async_trait::async_trait;
use tokio::sync::mpsc;

#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Run one model round. Assistant text is emitted incrementally on
    /// `events` as it streams in; the return value is the same content,
    /// fully assembled, for the agent loop to act on.
    async fn round(
        &self,
        model: &str,
        system: &str,
        messages: &[Message],
        tools: &[ToolDef],
        events: &mpsc::Sender<AgentEvent>,
    ) -> Result<RoundOutput, AgentError>;
}
