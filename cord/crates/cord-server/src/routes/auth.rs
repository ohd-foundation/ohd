//! `/v1/auth/*` — OIDC login (Authorization Code + PKCE) and logout.

use crate::errors::{ApiError, ApiResult};
use crate::oidc;
use crate::routes::{redirect_to, redirect_with_cookie};
use crate::server::AppState;
use crate::session;
use axum::body::Body;
use axum::extract::{Query, State};
use axum::http::header::{CONTENT_TYPE, SET_COOKIE};
use axum::http::StatusCode;
use axum::response::Response;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

/// The login screen lists these — id + issuer only, never secrets.
pub async fn providers(State(app): State<AppState>) -> Json<Value> {
    let providers: Vec<Value> = app
        .config
        .providers
        .iter()
        .map(|p| json!({ "id": p.id, "issuer": p.issuer }))
        .collect();
    Json(json!({ "providers": providers }))
}

#[derive(Deserialize)]
pub struct StartQuery {
    provider: String,
}

/// Begin a login: stash a PKCE verifier and 302 the browser to the IdP.
pub async fn start(
    State(app): State<AppState>,
    Query(q): Query<StartQuery>,
) -> ApiResult<Response> {
    let provider = app.config.provider(&q.provider).ok_or(ApiError::NotFound)?;
    let redirect_uri = format!("{}/v1/auth/callback", app.config.public_url);
    let (url, state, verifier) = app.oidc.authorize_url(provider, &redirect_uri).await?;
    oidc::insert_pending(&app.pending, &state, &provider.id, &verifier).await;
    Ok(redirect_to(&url))
}

#[derive(Deserialize)]
pub struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// IdP redirect target: exchange the code, verify the id_token, mint a
/// session cookie, and bounce the browser back to the SPA.
pub async fn callback(
    State(app): State<AppState>,
    Query(q): Query<CallbackQuery>,
) -> ApiResult<Response> {
    if let Some(e) = q.error {
        return Err(ApiError::Upstream(format!(
            "identity provider returned an error: {e}"
        )));
    }
    let code = q
        .code
        .ok_or_else(|| ApiError::BadRequest("callback is missing `code`".into()))?;
    let state = q
        .state
        .ok_or_else(|| ApiError::BadRequest("callback is missing `state`".into()))?;

    let pending = oidc::take_pending(&app.pending, &state)
        .await
        .ok_or(ApiError::Unauthorized)?;
    let provider = app
        .config
        .provider(&pending.provider_id)
        .ok_or(ApiError::NotFound)?;

    let redirect_uri = format!("{}/v1/auth/callback", app.config.public_url);
    let id_token = app
        .oidc
        .exchange_code(provider, &code, &pending.code_verifier, &redirect_uri)
        .await?;
    let verified = app.oidc.verify_id_token(provider, &id_token).await?;

    let user = app
        .db
        .upsert_user(&verified.iss, &verified.sub, Some(&verified.label()))?;
    let token = session::mint(
        &user.cord_user_ulid,
        &app.config.session_secret,
        app.config.session_ttl_hours,
    )?;
    let secure = app.config.public_url.starts_with("https");
    let cookie = session::cookie_for(&token, app.config.session_ttl_hours, secure);
    Ok(redirect_with_cookie(&app.config.public_url, &cookie))
}

/// Clear the session cookie.
pub async fn logout(State(app): State<AppState>) -> Response {
    let secure = app.config.public_url.starts_with("https");
    Response::builder()
        .status(StatusCode::OK)
        .header(SET_COOKIE, session::clear_cookie(secure))
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"ok":true}"#))
        .expect("static logout response is valid")
}
