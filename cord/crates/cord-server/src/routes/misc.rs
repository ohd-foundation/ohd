//! Liveness + the `/v1/me` identity-and-policy endpoint.

use crate::errors::ApiResult;
use crate::server::AppState;
use crate::session::CurrentUser;
use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

pub async fn healthz() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "cord-server",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// The current user plus the deployment policy flags the SPA needs to
/// render itself (which providers exist, whether BYO keys are allowed).
pub async fn me(user: CurrentUser, State(app): State<AppState>) -> ApiResult<Json<Value>> {
    let u = app.db.user(&user.0)?;
    Ok(Json(json!({
        "user": u,
        "policy": {
            "allow_user_keys": app.config.allow_user_keys,
            "allow_custom_relay": app.config.allow_custom_relay,
            "default_relay": app.config.default_relay,
            "default_model_provider": app.config.default_model_provider,
        },
    })))
}
