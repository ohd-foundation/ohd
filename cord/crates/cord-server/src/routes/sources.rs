//! `/v1/sources/*` — the data-source registry. A source is one share
//! credential: a sealed grant token plus how to reach the storage.

use crate::crypto;
use crate::db::NewSource;
use crate::errors::{ApiError, ApiResult};
use crate::server::AppState;
use crate::session::CurrentUser;
use crate::share;
use axum::extract::{Path, State};
use axum::Json;
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

/// Re-probe reachability. Direct sources get a real HTTP probe; relay
/// sources report `pending_relay` until the Phase 4 data plane lands.
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
        ("pending_relay", false)
    };
    app.db.set_source_status(&user.0, &id, status, ok)?;
    Ok(Json(json!({ "source": app.db.get_source(&user.0, &id)? })))
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
