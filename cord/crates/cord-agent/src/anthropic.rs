//! Anthropic Messages API provider. Non-streaming per round — the agent
//! still streams *events* (text per block, tool-use status) to the
//! caller; token-level streaming is a later refinement (see
//! `spec/deferred.md` "CORD chat polish").

use crate::provider::ModelProvider;
use crate::types::{AgentError, ContentBlock, Message, RoundOutput, ToolDef};
use async_trait::async_trait;
use serde_json::{json, Value};

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";
const MAX_TOKENS: u32 = 4096;

pub struct AnthropicProvider {
    http: reqwest::Client,
    api_key: String,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key: api_key.into(),
        }
    }
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    async fn round(
        &self,
        model: &str,
        system: &str,
        messages: &[Message],
        tools: &[ToolDef],
    ) -> Result<RoundOutput, AgentError> {
        let body = json!({
            "model": model,
            "max_tokens": MAX_TOKENS,
            "system": system,
            "tools": tools.iter().map(tool_json).collect::<Vec<_>>(),
            "messages": messages.iter().map(message_json).collect::<Vec<_>>(),
        });
        let resp = self
            .http
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AgentError::Provider(format!("request failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AgentError::Provider(format!("HTTP {status}: {text}")));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| AgentError::Provider(format!("response not JSON: {e}")))?;
        let blocks = parse_blocks(v.get("content"));
        let stop_reason = v
            .get("stop_reason")
            .and_then(|s| s.as_str())
            .unwrap_or("end_turn")
            .to_string();
        Ok(RoundOutput { blocks, stop_reason })
    }
}

fn tool_json(t: &ToolDef) -> Value {
    json!({
        "name": t.name,
        "description": t.description,
        "input_schema": t.input_schema,
    })
}

fn message_json(m: &Message) -> Value {
    json!({
        "role": m.role,
        "content": m.content.iter().map(block_json).collect::<Vec<_>>(),
    })
}

fn block_json(b: &ContentBlock) -> Value {
    match b {
        ContentBlock::Text(t) => json!({ "type": "text", "text": t }),
        ContentBlock::ToolUse { id, name, input } => {
            json!({ "type": "tool_use", "id": id, "name": name, "input": input })
        }
        ContentBlock::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => json!({
            "type": "tool_result",
            "tool_use_id": tool_use_id,
            "content": content,
            "is_error": is_error,
        }),
    }
}

fn parse_blocks(content: Option<&Value>) -> Vec<ContentBlock> {
    let Some(arr) = content.and_then(|c| c.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|b| match b.get("type").and_then(|t| t.as_str()) {
            Some("text") => Some(ContentBlock::Text(
                b.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string(),
            )),
            Some("tool_use") => Some(ContentBlock::ToolUse {
                id: b.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                name: b.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
                input: b.get("input").cloned().unwrap_or_else(|| json!({})),
            }),
            _ => None,
        })
        .collect()
}
