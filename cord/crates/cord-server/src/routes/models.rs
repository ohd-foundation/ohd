//! `/v1/models` — the model catalog and bring-your-own keys.

use crate::crypto;
use crate::errors::{ApiError, ApiResult};
use crate::server::AppState;
use crate::session::CurrentUser;
use axum::extract::{Path, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

/// System (deployment-wide) providers plus, when policy allows, the
/// user's own registered keys.
pub async fn list(user: CurrentUser, State(app): State<AppState>) -> ApiResult<Json<Value>> {
    let system: Vec<Value> = app
        .config
        .model_providers
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "kind": p.kind,
                "models": p.models,
                "has_key": !p.api_key.is_empty(),
            })
        })
        .collect();
    let byo: Vec<Value> = if app.config.allow_user_keys {
        app.db
            .list_byo(&user.0)?
            .into_iter()
            .map(|k| {
                json!({
                    "id": k.id,
                    "provider_kind": k.provider_kind,
                    "label": k.label,
                    "created_at": k.created_at,
                })
            })
            .collect()
    } else {
        Vec::new()
    };
    Ok(Json(json!({
        "system_providers": system,
        "default_provider": app.config.default_model_provider,
        "allow_user_keys": app.config.allow_user_keys,
        "byo_keys": byo,
    })))
}

#[derive(Deserialize)]
pub struct AddByoBody {
    provider_kind: String,
    label: String,
    api_key: String,
}

/// Register a user-supplied model key. Rejected outright when the
/// deployment disables BYO keys (the HIPAA-compliance lever).
pub async fn add_byo(
    user: CurrentUser,
    State(app): State<AppState>,
    Json(body): Json<AddByoBody>,
) -> ApiResult<Json<Value>> {
    if !app.config.allow_user_keys {
        return Err(ApiError::Forbidden(
            "this deployment does not allow user-supplied model keys".into(),
        ));
    }
    if body.api_key.trim().is_empty() {
        return Err(ApiError::BadRequest("`api_key` is empty".into()));
    }
    let kind = body.provider_kind.to_lowercase();
    if !matches!(kind.as_str(), "anthropic" | "gemini" | "openai") {
        return Err(ApiError::BadRequest(format!(
            "unknown provider kind `{kind}`"
        )));
    }
    let sealed = crypto::seal_str(&app.config.data_key, &body.api_key);
    let key = app.db.insert_byo(&user.0, &kind, &body.label, &sealed)?;
    Ok(Json(json!({
        "key": {
            "id": key.id,
            "provider_kind": key.provider_kind,
            "label": key.label,
            "created_at": key.created_at,
        }
    })))
}

pub async fn delete_byo(
    user: CurrentUser,
    State(app): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Value>> {
    app.db.delete_byo(&user.0, &id)?;
    Ok(Json(json!({ "ok": true })))
}
