//! The tool-use loop. Owns the "ask the model → run any tools it asked
//! for → loop" cycle, streaming [`AgentEvent`]s as it goes. Bounded by
//! [`MAX_ROUNDS`] so a misbehaving model can't churn forever.

use crate::mcp::McpClient;
use crate::provider::ModelProvider;
use crate::types::{AgentEvent, ContentBlock, Message};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;

const MAX_ROUNDS: usize = 8;

pub const SYSTEM_PROMPT: &str = "\
You are CORD, the OHD health-data agent. You answer questions about a \
person's own health and lifestyle data by calling tools against their \
connected data source.

Workflow:
  1. If you don't yet know what data exists, call `describe_data` first.
  2. For any time-relative question (\"today\", \"last week\"), call `now` \
     once to anchor the clock and time zone.
  3. Use the read tools to fetch only what you need.

Access is scoped by the share the user granted CORD. A \"permission \
denied\" or out-of-scope result means the user chose not to share that \
data — say so plainly; never present it as missing data. Keep replies \
short and concrete, quoting real numbers from the tools you called.";

/// Drives one conversation against one data source.
pub struct Agent {
    provider: Arc<dyn ModelProvider>,
    model: String,
    mcp: McpClient,
}

impl Agent {
    pub fn new(provider: Arc<dyn ModelProvider>, model: impl Into<String>, mcp: McpClient) -> Self {
        Self {
            provider,
            model: model.into(),
            mcp,
        }
    }

    /// Run the loop for one user turn. `history` is the whole conversation
    /// so far, oldest first, ending with the user's new message. Events
    /// are sent on `tx`; the loop always ends with `Done` or `Error`.
    pub async fn run(&self, mut messages: Vec<Message>, tx: mpsc::Sender<AgentEvent>) {
        let tools = match self.mcp.list_tools().await {
            Ok(t) => t,
            Err(e) => {
                let _ = tx
                    .send(AgentEvent::Error {
                        message: format!("could not reach the data source: {e}"),
                    })
                    .await;
                return;
            }
        };

        for round in 0..MAX_ROUNDS {
            // The provider streams assistant `Text` deltas onto `tx` as
            // they arrive; the returned `RoundOutput` is the same content
            // assembled, which the loop below acts on for tool use.
            let out = match self
                .provider
                .round(&self.model, SYSTEM_PROMPT, &messages, &tools, &tx)
                .await
            {
                Ok(o) => o,
                Err(e) => {
                    let _ = tx.send(AgentEvent::Error { message: e.to_string() }).await;
                    return;
                }
            };

            if out.stop_reason != "tool_use" {
                let _ = tx.send(AgentEvent::Done).await;
                return;
            }

            messages.push(Message {
                role: "assistant".into(),
                content: out.blocks.clone(),
            });

            let mut results = Vec::new();
            for block in &out.blocks {
                let ContentBlock::ToolUse { id, name, input } = block else {
                    continue;
                };
                let _ = tx.send(AgentEvent::Tool { name: name.clone() }).await;
                let (content, is_error) = match self.mcp.call_tool(name, input.clone()).await {
                    Ok(pair) => pair,
                    Err(e) => (json!({ "error": e.to_string() }).to_string(), true),
                };
                results.push(ContentBlock::ToolResult {
                    tool_use_id: id.clone(),
                    content,
                    is_error,
                });
            }

            if results.is_empty() {
                let _ = tx.send(AgentEvent::Done).await;
                return;
            }
            messages.push(Message {
                role: "user".into(),
                content: results,
            });

            tracing::debug!(round = round + 1, "completed a tool round");
        }

        let _ = tx
            .send(AgentEvent::Text {
                delta: format!("(Stopped after {MAX_ROUNDS} tool rounds — try rephrasing.)"),
            })
            .await;
        let _ = tx.send(AgentEvent::Done).await;
    }
}
