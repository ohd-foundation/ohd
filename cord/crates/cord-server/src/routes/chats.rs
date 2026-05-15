//! `/v1/chats/*` — conversations. `send_message` runs the `cord-agent`
//! tool-use loop and streams the result back as Server-Sent Events.

use crate::crypto;
use crate::errors::{ApiError, ApiResult};
use crate::server::AppState;
use crate::session::CurrentUser;
use axum::extract::{Path, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use cord_agent::{Agent, AgentEvent, AnthropicProvider, McpClient, Message, ModelProvider};
use serde::Deserialize;
use serde_json::{json, Value};
use std::convert::Infallible;
use std::sync::Arc;

pub async fn list(user: CurrentUser, State(app): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "chats": app.db.list_chats(&user.0)? })))
}

#[derive(Deserialize)]
pub struct CreateChatBody {
    source_id: String,
    model: Option<String>,
}

pub async fn create(
    user: CurrentUser,
    State(app): State<AppState>,
    Json(body): Json<CreateChatBody>,
) -> ApiResult<Json<Value>> {
    // Confirms the source exists and belongs to this user.
    app.db.get_source(&user.0, &body.source_id)?;
    let model = body
        .model
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| app.config.default_model_provider.clone());
    let chat = app.db.insert_chat(&user.0, &body.source_id, &model)?;
    Ok(Json(json!({ "chat": chat })))
}

pub async fn get_one(
    user: CurrentUser,
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let chat = app.db.get_chat(&user.0, &id)?;
    let messages = app.db.list_messages(&chat.id)?;
    Ok(Json(json!({ "chat": chat, "messages": messages })))
}

pub async fn delete_one(
    user: CurrentUser,
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    app.db.delete_chat(&user.0, &id)?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
pub struct SendMessageBody {
    message: String,
}

/// Send a user message; the response is a `text/event-stream` of
/// [`AgentEvent`]s. The user message is persisted before the agent runs;
/// the accumulated assistant text is persisted when the stream ends.
pub async fn send_message(
    user: CurrentUser,
    State(app): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SendMessageBody>,
) -> ApiResult<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>> {
    if body.message.trim().is_empty() {
        return Err(ApiError::BadRequest("message is empty".into()));
    }
    let chat = app.db.get_chat(&user.0, &id)?;
    let source = app.db.get_source(&user.0, &chat.source_id)?;

    let (provider, model_name) = resolve_provider(&app, &chat.model)?;

    let token = crypto::unseal_str(&app.config.data_key, &source.enc_token)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("could not unseal source token: {e}")))?;
    let mcp = McpClient::new(source.endpoint.clone(), Some(token));

    // Persist the user turn, then load the whole conversation as agent
    // messages (the list now ends with this turn).
    app.db.insert_message(&chat.id, "user", &body.message)?;
    let history: Vec<Message> = app
        .db
        .list_messages(&chat.id)?
        .into_iter()
        .map(|m| match m.role.as_str() {
            "assistant" => Message::assistant_text(m.content),
            _ => Message::user(m.content),
        })
        .collect();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<AgentEvent>(32);
    let agent = Agent::new(provider, model_name, mcp);
    tokio::spawn(async move {
        agent.run(history, tx).await;
    });

    let db = app.db.clone();
    let chat_id = chat.id.clone();
    let stream = async_stream::stream! {
        let mut assistant = String::new();
        while let Some(ev) = rx.recv().await {
            if let AgentEvent::Text { delta } = &ev {
                assistant.push_str(delta);
            }
            let terminal = matches!(ev, AgentEvent::Done | AgentEvent::Error { .. });
            let event = Event::default()
                .json_data(&ev)
                .unwrap_or_else(|_| Event::default().data("{}"));
            yield Ok::<_, Infallible>(event);
            if terminal {
                if !assistant.trim().is_empty() {
                    let _ = db.insert_message(&chat_id, "assistant", assistant.trim());
                }
                break;
            }
        }
    };

    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

/// Resolve a chat's `model` (a configured provider id) to a live
/// [`ModelProvider`] + the concrete model name to call. Phase 2 wires
/// Anthropic; other provider kinds are an honest 501.
fn resolve_provider(
    app: &AppState,
    provider_id: &str,
) -> ApiResult<(Arc<dyn ModelProvider>, String)> {
    let cfg = app.config.model_provider(provider_id).ok_or_else(|| {
        ApiError::BadRequest(format!("unknown model provider `{provider_id}`"))
    })?;
    if cfg.api_key.is_empty() {
        return Err(ApiError::BadRequest(format!(
            "model provider `{}` has no API key configured",
            cfg.id
        )));
    }
    let model_name = cfg
        .models
        .first()
        .cloned()
        .unwrap_or_else(|| "claude-sonnet-4-6".to_string());
    let provider: Arc<dyn ModelProvider> = match cfg.kind.as_str() {
        "anthropic" => Arc::new(AnthropicProvider::new(cfg.api_key.clone())),
        other => {
            return Err(ApiError::NotImplemented(format!(
                "model provider kind `{other}` is not wired yet (Phase 2 ships Anthropic)"
            )))
        }
    };
    Ok((provider, model_name))
}
