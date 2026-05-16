//! Anthropic Messages API provider — true token-level streaming.
//!
//! Each round calls the Messages API with `stream: true` and parses the
//! SSE event stream. `text_delta` events are emitted as `AgentEvent::Text`
//! deltas the moment they arrive; `input_json_delta` fragments are
//! accumulated into each `tool_use` block's input JSON. The assembled
//! [`RoundOutput`] (with the streamed `stop_reason`) is returned when the
//! `message_stop` event lands.

use crate::provider::ModelProvider;
use crate::types::{AgentError, AgentEvent, ContentBlock, Message, RoundOutput, ToolDef};
use async_trait::async_trait;
use futures_util::StreamExt;
use serde_json::{json, Value};
use tokio::sync::mpsc;

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
        events: &mpsc::Sender<AgentEvent>,
    ) -> Result<RoundOutput, AgentError> {
        let body = json!({
            "model": model,
            "max_tokens": MAX_TOKENS,
            "system": system,
            "stream": true,
            "tools": tools.iter().map(tool_json).collect::<Vec<_>>(),
            "messages": messages.iter().map(message_json).collect::<Vec<_>>(),
        });
        let resp = self
            .http
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .header("accept", "text/event-stream")
            .json(&body)
            .send()
            .await
            .map_err(|e| AgentError::Provider(format!("request failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(AgentError::Provider(format!("HTTP {status}: {text}")));
        }

        let mut parser = StreamParser::new();
        let mut bytes = resp.bytes_stream();
        while let Some(chunk) = bytes.next().await {
            let chunk =
                chunk.map_err(|e| AgentError::Provider(format!("stream read failed: {e}")))?;
            parser.feed(&chunk);
            // Each completed SSE event yields its `data:` JSON; apply it.
            while let Some(data) = parser.next_event() {
                let Ok(value) = serde_json::from_str::<Value>(&data) else {
                    continue;
                };
                parser.state.apply(&value, events).await;
            }
        }

        Ok(parser.state.finish())
    }
}

/// Splits the raw SSE byte stream into per-event `data:` payloads. SSE
/// events are blank-line delimited; we only care about the `data:` lines.
struct StreamParser {
    buffer: String,
    pending: Vec<String>,
    state: StreamState,
}

impl StreamParser {
    fn new() -> Self {
        Self {
            buffer: String::new(),
            pending: Vec::new(),
            state: StreamState::default(),
        }
    }

    fn feed(&mut self, chunk: &[u8]) {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        while let Some(idx) = self.buffer.find("\n\n") {
            let raw = self.buffer[..idx].to_string();
            self.buffer.drain(..idx + 2);
            let mut data = String::new();
            for line in raw.split('\n') {
                if let Some(rest) = line.strip_prefix("data:") {
                    if !data.is_empty() {
                        data.push('\n');
                    }
                    data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
                }
            }
            if !data.is_empty() {
                self.pending.push(data);
            }
        }
    }

    fn next_event(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            None
        } else {
            Some(self.pending.remove(0))
        }
    }
}

/// In-progress assistant blocks, indexed by their content-block `index`.
/// A `text` block accumulates `text_delta`s; a `tool_use` block
/// accumulates `input_json_delta` fragments into a JSON string.
#[derive(Default)]
struct StreamState {
    blocks: Vec<BlockBuilder>,
    stop_reason: Option<String>,
}

enum BlockBuilder {
    Text(String),
    ToolUse { id: String, name: String, json: String },
}

impl StreamState {
    async fn apply(&mut self, value: &Value, events: &mpsc::Sender<AgentEvent>) {
        match value.get("type").and_then(|t| t.as_str()) {
            Some("content_block_start") => {
                let idx = value.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                let block = value.get("content_block");
                let builder = match block.and_then(|b| b.get("type")).and_then(|t| t.as_str()) {
                    Some("tool_use") => BlockBuilder::ToolUse {
                        id: str_field(block, "id"),
                        name: str_field(block, "name"),
                        json: String::new(),
                    },
                    _ => BlockBuilder::Text(String::new()),
                };
                set_at(&mut self.blocks, idx, builder);
            }
            Some("content_block_delta") => {
                let idx = value.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                let delta = value.get("delta");
                match delta.and_then(|d| d.get("type")).and_then(|t| t.as_str()) {
                    Some("text_delta") => {
                        let text = str_field(delta, "text");
                        if let Some(BlockBuilder::Text(buf)) = self.blocks.get_mut(idx) {
                            buf.push_str(&text);
                        }
                        if !text.is_empty() {
                            let _ = events.send(AgentEvent::Text { delta: text }).await;
                        }
                    }
                    Some("input_json_delta") => {
                        let partial = str_field(delta, "partial_json");
                        if let Some(BlockBuilder::ToolUse { json, .. }) = self.blocks.get_mut(idx) {
                            json.push_str(&partial);
                        }
                    }
                    _ => {}
                }
            }
            Some("message_delta") => {
                if let Some(reason) = value
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(|s| s.as_str())
                {
                    self.stop_reason = Some(reason.to_string());
                }
            }
            // `message_start`, `content_block_stop`, `message_stop`, `ping`
            // need no extra handling — block assembly is delta-driven.
            _ => {}
        }
    }

    fn finish(self) -> RoundOutput {
        let blocks = self
            .blocks
            .into_iter()
            .map(|b| match b {
                BlockBuilder::Text(t) => ContentBlock::Text(t),
                BlockBuilder::ToolUse { id, name, json } => ContentBlock::ToolUse {
                    id,
                    name,
                    input: serde_json::from_str(&json).unwrap_or_else(|_| json!({})),
                },
            })
            .collect();
        RoundOutput {
            blocks,
            stop_reason: self.stop_reason.unwrap_or_else(|| "end_turn".to_string()),
        }
    }
}

/// Place `builder` at `idx`, growing the vec with empty text blocks if the
/// stream reports a higher index than we have seen.
fn set_at(blocks: &mut Vec<BlockBuilder>, idx: usize, builder: BlockBuilder) {
    while blocks.len() <= idx {
        blocks.push(BlockBuilder::Text(String::new()));
    }
    blocks[idx] = builder;
}

fn str_field(v: Option<&Value>, key: &str) -> String {
    v.and_then(|v| v.get(key))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string()
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

#[cfg(test)]
mod tests {
    use super::*;

    // Drives a StreamState through a canned SSE sequence and checks the
    // assembled output + that text deltas were emitted incrementally.
    #[tokio::test]
    async fn assembles_text_and_tool_use_from_sse_events() {
        let (tx, mut rx) = mpsc::channel(32);
        let mut state = StreamState::default();
        let events: &[Value] = &[
            json!({ "type": "message_start" }),
            json!({ "type": "content_block_start", "index": 0,
                    "content_block": { "type": "text", "text": "" } }),
            json!({ "type": "content_block_delta", "index": 0,
                    "delta": { "type": "text_delta", "text": "Hel" } }),
            json!({ "type": "content_block_delta", "index": 0,
                    "delta": { "type": "text_delta", "text": "lo" } }),
            json!({ "type": "content_block_stop", "index": 0 }),
            json!({ "type": "content_block_start", "index": 1,
                    "content_block": { "type": "tool_use", "id": "t1", "name": "now" } }),
            json!({ "type": "content_block_delta", "index": 1,
                    "delta": { "type": "input_json_delta", "partial_json": "{\"tz\":" } }),
            json!({ "type": "content_block_delta", "index": 1,
                    "delta": { "type": "input_json_delta", "partial_json": "\"utc\"}" } }),
            json!({ "type": "content_block_stop", "index": 1 }),
            json!({ "type": "message_delta", "delta": { "stop_reason": "tool_use" } }),
            json!({ "type": "message_stop" }),
        ];
        for ev in events {
            state.apply(ev, &tx).await;
        }
        drop(tx);

        let mut deltas = Vec::new();
        while let Some(ev) = rx.recv().await {
            if let AgentEvent::Text { delta } = ev {
                deltas.push(delta);
            }
        }
        assert_eq!(deltas, vec!["Hel", "lo"]);

        let out = state.finish();
        assert_eq!(out.stop_reason, "tool_use");
        assert_eq!(out.blocks.len(), 2);
        match &out.blocks[0] {
            ContentBlock::Text(t) => assert_eq!(t, "Hello"),
            _ => panic!("expected text block"),
        }
        match &out.blocks[1] {
            ContentBlock::ToolUse { id, name, input } => {
                assert_eq!(id, "t1");
                assert_eq!(name, "now");
                assert_eq!(input, &json!({ "tz": "utc" }));
            }
            _ => panic!("expected tool_use block"),
        }
    }

    #[test]
    fn parser_splits_sse_data_lines() {
        let mut parser = StreamParser::new();
        parser.feed(b"event: message_start\ndata: {\"type\":\"message_start\"}\n\n");
        parser.feed(b"event: ping\ndata: {\"ty");
        parser.feed(b"pe\":\"ping\"}\n\n");
        assert_eq!(
            parser.next_event().as_deref(),
            Some("{\"type\":\"message_start\"}"),
        );
        assert_eq!(parser.next_event().as_deref(), Some("{\"type\":\"ping\"}"));
        assert_eq!(parser.next_event(), None);
    }

    #[test]
    fn finish_defaults_stop_reason_when_unset() {
        let out = StreamState::default().finish();
        assert_eq!(out.stop_reason, "end_turn");
        assert!(out.blocks.is_empty());
    }
}
