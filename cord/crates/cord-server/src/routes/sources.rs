//! `/v1/sources/*` — the data-source registry. A source is one share
//! credential: a sealed grant token plus how to reach the storage.

use crate::crypto;
use crate::db::{DataSource, NewSource};
use crate::errors::{ApiError, ApiResult};
use crate::server::AppState;
use crate::session::CurrentUser;
use crate::share;
use axum::extract::{Path, State};
use axum::Json;
use cord_agent::{McpClient, RelayTarget};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;

pub async fn list(user: CurrentUser, State(app): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "sources": app.db.list_sources(&user.0)? })))
}

#[derive(Deserialize)]
pub struct ConnectBody {
    label: Option<String>,
    /// A share link: `ohd://share/<rid>?token=…&pin=…&relay=…`.
    link: Option<String>,
    /// Direct mode (a CA-cert storage URL, no relay): explicit fields.
    endpoint: Option<String>,
    token: Option<String>,
    pin: Option<String>,
}

/// Connect a data source — either by share link (relay-bound) or by an
/// explicit direct storage URL.
pub async fn connect(
    user: CurrentUser,
    State(app): State<AppState>,
    Json(body): Json<ConnectBody>,
) -> ApiResult<Json<Value>> {
    let new = if let Some(link) = body.link.as_deref() {
        let parsed = share::parse_share_link(link)?;
        NewSource {
            label: body.label.unwrap_or_else(|| "Shared storage".into()),
            kind: "relay".into(),
            endpoint: format!(
                "{}/r/{}",
                parsed.relay_host.trim_end_matches('/'),
                parsed.rendezvous_id
            ),
            rendezvous_id: Some(parsed.rendezvous_id),
            relay_host: Some(parsed.relay_host),
            enc_token: crypto::seal_str(&app.config.data_key, &parsed.token),
            cert_pin: parsed.pin,
            scope_json: None,
        }
    } else {
        let endpoint = body.endpoint.ok_or_else(|| {
            ApiError::BadRequest("provide a `link`, or `endpoint` + `token`".into())
        })?;
        let token = body
            .token
            .ok_or_else(|| ApiError::BadRequest("a direct source requires `token`".into()))?;
        NewSource {
            label: body.label.unwrap_or_else(|| "Direct storage".into()),
            kind: "direct".into(),
            endpoint,
            rendezvous_id: None,
            relay_host: None,
            enc_token: crypto::seal_str(&app.config.data_key, &token),
            cert_pin: body.pin,
            scope_json: None,
        }
    };
    Ok(Json(json!({ "source": app.db.insert_source(&user.0, new)? })))
}

pub async fn get_one(
    user: CurrentUser,
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "source": app.db.get_source(&user.0, &id)? })))
}

pub async fn delete_one(
    user: CurrentUser,
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    app.db.delete_source(&user.0, &id)?;
    Ok(Json(json!({ "ok": true })))
}

/// Body for `PATCH /v1/sources/:id` — only the UI label is editable.
#[derive(Deserialize)]
pub struct RenameBody {
    pub label: String,
}

/// Rename a source's UI label. Doesn't touch credentials, endpoint, scope,
/// or reachability — pure presentation. Trims whitespace; rejects empty.
pub async fn rename(
    user: CurrentUser,
    State(app): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<RenameBody>,
) -> ApiResult<Json<Value>> {
    let label = body.label.trim();
    if label.is_empty() {
        return Err(ApiError::BadRequest("label cannot be empty".into()));
    }
    if label.len() > 200 {
        return Err(ApiError::BadRequest("label too long (max 200 chars)".into()));
    }
    app.db.rename_source(&user.0, &id, label)?;
    Ok(Json(json!({ "source": app.db.get_source(&user.0, &id)? })))
}

/// Re-probe reachability. A `direct` source gets a real HTTP probe; a
/// `relay` source gets a real relay-tunnel reachability check — open the
/// tunnel, complete the pinned inner-TLS handshake, run MCP `initialize`.
pub async fn refresh(
    user: CurrentUser,
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let source = app.db.get_source(&user.0, &id)?;
    let (status, ok) = if source.kind == "direct" {
        if probe(&source.endpoint).await {
            ("connected", true)
        } else {
            ("unreachable", false)
        }
    } else {
        // Relay source: a reachability check is the full tunnel open.
        // `McpClient::probe` runs relay attach + pinned inner-TLS + MCP
        // `initialize`. A pin mismatch, an offline phone, or a bad token
        // all surface here as `unreachable`.
        match build_mcp_client(&app, &source) {
            Ok(mcp) => match mcp.probe().await {
                Ok(()) => ("connected", true),
                Err(e) => {
                    tracing::info!(source = %id, error = %e, "relay source unreachable");
                    ("unreachable", false)
                }
            },
            Err(e) => {
                tracing::warn!(source = %id, error = %e, "could not build relay client");
                ("unreachable", false)
            }
        }
    };
    app.db.set_source_status(&user.0, &id, status, ok)?;
    Ok(Json(json!({ "source": app.db.get_source(&user.0, &id)? })))
}

/// A compact, read-only data summary for a connected source.
///
/// Calls the source's MCP `describe_data` tool and returns its parsed JSON
/// under `summary`. An offline (phone-backed) connection is not a server
/// error: when the source is unreachable or the tool fails this returns
/// HTTP 200 with `{ "summary": null, "status": "unreachable" }`.
pub async fn summary(
    user: CurrentUser,
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    let source = app.db.get_source(&user.0, &id)?;
    let unreachable = || Json(json!({ "summary": null, "status": "unreachable" }));

    let mcp = match build_mcp_client(&app, &source) {
        Ok(mcp) => mcp,
        Err(e) => {
            tracing::info!(source = %id, error = %e, "summary: could not build mcp client");
            return Ok(unreachable());
        }
    };
    match mcp.call_tool("describe_data", json!({})).await {
        Ok((text, false)) => match serde_json::from_str::<Value>(&text) {
            Ok(parsed) => Ok(Json(json!({ "summary": parsed }))),
            Err(e) => {
                tracing::info!(source = %id, error = %e, "summary: describe_data output not JSON");
                Ok(unreachable())
            }
        },
        Ok((text, true)) => {
            tracing::info!(source = %id, tool_error = %text, "summary: describe_data tool error");
            Ok(unreachable())
        }
        Err(e) => {
            tracing::info!(source = %id, error = %e, "summary: describe_data unreachable");
            Ok(unreachable())
        }
    }
}

async fn probe(endpoint: &str) -> bool {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    else {
        return false;
    };
    client
        .get(endpoint)
        .send()
        .await
        .map(|r| r.status().as_u16() < 500)
        .unwrap_or(false)
}

/// Build a transport-correct [`McpClient`] for a stored data source.
///
/// `direct` sources get the plain-HTTP transport against `endpoint`;
/// `relay` sources get the relay-tunnelled transport built from the
/// rendezvous id, relay host, unsealed grant token, and cert pin the
/// share link carried. Used by both the chat data plane and the
/// reachability check.
pub(crate) fn build_mcp_client(
    app: &AppState,
    source: &DataSource,
) -> Result<McpClient, ApiError> {
    let token = crypto::unseal_str(&app.config.data_key, &source.enc_token)
        .map_err(|e| ApiError::Internal(anyhow::anyhow!("could not unseal source token: {e}")))?;
    if source.kind == "relay" {
        let rendezvous_id = source.rendezvous_id.clone().ok_or_else(|| {
            ApiError::Internal(anyhow::anyhow!("relay source is missing its rendezvous id"))
        })?;
        let relay_host = source.relay_host.clone().ok_or_else(|| {
            ApiError::Internal(anyhow::anyhow!("relay source is missing its relay host"))
        })?;
        // A relay-bound (phone) storage uses a self-signed identity cert:
        // the pin is the entire trust anchor. Refuse to dial without it.
        let pin = source.cert_pin.clone().ok_or_else(|| {
            ApiError::BadRequest(
                "relay source has no cert pin — re-connect with a share link that carries `pin`"
                    .into(),
            )
        })?;
        Ok(McpClient::relay(RelayTarget {
            relay_host,
            rendezvous_id,
            pin,
            token,
        }))
    } else {
        Ok(McpClient::new(source.endpoint.clone(), Some(token)))
    }
}
