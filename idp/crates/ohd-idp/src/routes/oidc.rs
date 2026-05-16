//! The OIDC authorization-code flow: `/authorize`, the SSR login + sign-up
//! UI, `/token`, and `/userinfo`.
//!
//! The flow (see `SPEC.md` — "The login flow"):
//!
//! 1. An RP sends the browser to `GET /authorize` with `client_id`,
//!    `redirect_uri`, `response_type=code`, `scope`, PKCE
//!    `code_challenge` + `code_challenge_method=S256`, `state`, `nonce`.
//! 2. The IdP validates the client + redirect URI against the registry.
//!    No session yet → it renders `GET /login`.
//! 3. `POST /login` verifies the email/password against the SaaS account
//!    store; `POST /signup` creates a new account and shows the recovery
//!    code once.
//! 4. On success the IdP mints a single-use authorization code and
//!    redirects the browser to `redirect_uri?code=…&state=…`.
//! 5. The RP calls `POST /token` with the code + PKCE `code_verifier`;
//!    the IdP returns the `id_token` + access/refresh tokens.
//! 6. `GET /userinfo` resolves a bearer access token to the user.

use crate::codes::{verify_pkce_s256, CodeError, Continuation};
use crate::errors::{ApiError, ApiResult};
use crate::html;
use crate::server::AppState;
use crate::token::{mint_id_token, ACCESS_TOKEN_TTL_SECS};
use axum::extract::{Form, Query, State};
use axum::http::header::{HeaderMap, AUTHORIZATION, LOCATION};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

// --- /authorize ------------------------------------------------------------

/// Query parameters of `GET /authorize`.
#[derive(Debug, Deserialize)]
pub struct AuthorizeParams {
    pub client_id: Option<String>,
    pub redirect_uri: Option<String>,
    pub response_type: Option<String>,
    pub scope: Option<String>,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
}

/// A fully-validated authorize request — every field present and sane.
struct ValidAuthorize {
    client_id: String,
    redirect_uri: String,
    scope: String,
    state: String,
    nonce: Option<String>,
    code_challenge: String,
}

/// Validate an authorize request against the client registry + the OIDC
/// rules. Two failure shapes:
///
/// - **Untrusted client / redirect** — `Err(html_error)`: the IdP must
///   render an error page, never redirect (a bad `redirect_uri` cannot be
///   trusted as a redirect target).
/// - **Bad protocol parameter** with a *trusted* redirect — the spec lets
///   an OP redirect an OAuth error back; for Phase 2 we keep it simple and
///   also render the error page. Either way the user is never sent on.
fn validate_authorize(
    app: &AppState,
    p: &AuthorizeParams,
) -> Result<ValidAuthorize, String> {
    let client_id = p
        .client_id
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("Missing client_id.")?;
    let client = app
        .clients
        .get(client_id)
        .ok_or("Unknown client_id — this application is not registered.")?;

    let redirect_uri = p
        .redirect_uri
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("Missing redirect_uri.")?;
    if !client.allows_redirect(redirect_uri) {
        return Err("redirect_uri is not registered for this client.".to_string());
    }

    // Past this point the redirect_uri is trusted.
    if p.response_type.as_deref() != Some("code") {
        return Err("Unsupported response_type — only `code` is supported.".to_string());
    }
    let scope = p.scope.as_deref().unwrap_or("");
    if !scope.split_whitespace().any(|s| s == "openid") {
        return Err("The `openid` scope is required.".to_string());
    }
    let code_challenge = p
        .code_challenge
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("PKCE code_challenge is required.")?;
    if p.code_challenge_method.as_deref() != Some("S256") {
        return Err("code_challenge_method must be S256 (PKCE is mandatory).".to_string());
    }
    let state = p
        .state
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or("Missing state.")?;

    Ok(ValidAuthorize {
        client_id: client_id.to_string(),
        redirect_uri: redirect_uri.to_string(),
        scope: scope.to_string(),
        state: state.to_string(),
        nonce: p.nonce.clone().filter(|s| !s.is_empty()),
        code_challenge: code_challenge.to_string(),
    })
}

/// `GET /authorize` — the OIDC authorization endpoint. With no session
/// (Phase 2 has no SSO cookie yet) a valid request renders the login page.
pub async fn authorize(
    State(app): State<AppState>,
    Query(p): Query<AuthorizeParams>,
) -> Response {
    match validate_authorize(&app, &p) {
        Ok(v) => Html(html::login_page(
            &v.client_id,
            &v.redirect_uri,
            &v.scope,
            &v.state,
            v.nonce.as_deref(),
            &v.code_challenge,
            app.config.signup.open,
            None,
        ))
        .into_response(),
        Err(msg) => {
            (StatusCode::BAD_REQUEST, Html(html::error_page(&msg))).into_response()
        }
    }
}

// --- /login ----------------------------------------------------------------

/// The authorize-flow parameters threaded through every login/sign-up form.
#[derive(Debug, Deserialize, Clone)]
pub struct FlowFields {
    pub client_id: String,
    pub redirect_uri: String,
    #[serde(default)]
    pub scope: String,
    pub state: String,
    #[serde(default)]
    pub nonce: Option<String>,
    pub code_challenge: String,
}

impl FlowFields {
    /// Re-validate the carried params against the registry — a form POST
    /// is as untrusted as the original query string.
    fn revalidate(&self, app: &AppState) -> Result<(), String> {
        let client = app
            .clients
            .get(&self.client_id)
            .ok_or("Unknown client_id.")?;
        if !client.allows_redirect(&self.redirect_uri) {
            return Err("redirect_uri is not registered for this client.".to_string());
        }
        if self.code_challenge.is_empty() {
            return Err("PKCE code_challenge is required.".to_string());
        }
        if !self.scope.split_whitespace().any(|s| s == "openid") {
            return Err("The `openid` scope is required.".to_string());
        }
        Ok(())
    }

    fn nonce(&self) -> Option<&str> {
        self.nonce.as_deref().filter(|s| !s.is_empty())
    }
}

/// `GET /login` — render the login form. Reachable directly (the sign-up
/// page links back here) as well as from `/authorize`.
pub async fn login_form(
    State(app): State<AppState>,
    Query(f): Query<FlowFields>,
) -> Response {
    if let Err(msg) = f.revalidate(&app) {
        return (StatusCode::BAD_REQUEST, Html(html::error_page(&msg))).into_response();
    }
    Html(html::login_page(
        &f.client_id,
        &f.redirect_uri,
        &f.scope,
        &f.state,
        f.nonce(),
        &f.code_challenge,
        app.config.signup.open,
        None,
    ))
    .into_response()
}

/// `POST /login` form body — the flow fields plus the credentials.
#[derive(Debug, Deserialize)]
pub struct LoginForm {
    #[serde(flatten)]
    pub flow: FlowFields,
    pub email: String,
    pub password: String,
}

/// `POST /login` — verify the credentials, then continue the authorize
/// flow by minting a code and redirecting. On failure re-render `/login`
/// with an error banner.
pub async fn login_submit(
    State(app): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Response {
    let f = &form.flow;
    if let Err(msg) = f.revalidate(&app) {
        return (StatusCode::BAD_REQUEST, Html(html::error_page(&msg))).into_response();
    }

    let account = match app.accounts.find_by_email(&form.email) {
        Ok(Some(a)) => a,
        Ok(None) => return login_error(&app, f),
        Err(e) => return ApiError::from(e).into_response(),
    };
    if !account.verify_password(&form.password) {
        return login_error(&app, f);
    }

    // Authenticated — mint a code and redirect back to the RP.
    let auth_time = now_unix();
    match issue_and_redirect(&app, f, &account.profile_ulid, &account.email, auth_time) {
        Ok(resp) => resp,
        Err(e) => e.into_response(),
    }
}

/// Re-render the login page with the generic credential error. The same
/// message for "no such email" and "wrong password" — no account
/// enumeration.
fn login_error(app: &AppState, f: &FlowFields) -> Response {
    Html(html::login_page(
        &f.client_id,
        &f.redirect_uri,
        &f.scope,
        &f.state,
        f.nonce(),
        &f.code_challenge,
        app.config.signup.open,
        Some("Incorrect email or password."),
    ))
    .into_response()
}

// --- /signup ---------------------------------------------------------------

/// `GET /signup` — render the sign-up form (404-equivalent when sign-up is
/// closed).
pub async fn signup_form(
    State(app): State<AppState>,
    Query(f): Query<FlowFields>,
) -> Response {
    if !app.config.signup.open {
        return (
            StatusCode::FORBIDDEN,
            Html(html::error_page("Self-service sign-up is disabled.")),
        )
            .into_response();
    }
    if let Err(msg) = f.revalidate(&app) {
        return (StatusCode::BAD_REQUEST, Html(html::error_page(&msg))).into_response();
    }
    Html(html::signup_page(
        &f.client_id,
        &f.redirect_uri,
        &f.scope,
        &f.state,
        f.nonce(),
        &f.code_challenge,
        None,
    ))
    .into_response()
}

/// `POST /signup` form body.
#[derive(Debug, Deserialize)]
pub struct SignupForm {
    #[serde(flatten)]
    pub flow: FlowFields,
    pub email: String,
    pub password: String,
    pub confirm: String,
}

/// `POST /signup` — create the account, then show the recovery code once
/// before continuing the authorize flow.
pub async fn signup_submit(
    State(app): State<AppState>,
    Form(form): Form<SignupForm>,
) -> Response {
    if !app.config.signup.open {
        return (
            StatusCode::FORBIDDEN,
            Html(html::error_page("Self-service sign-up is disabled.")),
        )
            .into_response();
    }
    let f = &form.flow;
    if let Err(msg) = f.revalidate(&app) {
        return (StatusCode::BAD_REQUEST, Html(html::error_page(&msg))).into_response();
    }

    let signup_error = |msg: &str| -> Response {
        Html(html::signup_page(
            &f.client_id,
            &f.redirect_uri,
            &f.scope,
            &f.state,
            f.nonce(),
            &f.code_challenge,
            Some(msg),
        ))
        .into_response()
    };

    if form.password != form.confirm {
        return signup_error("The two passwords do not match.");
    }

    let created = match app.accounts.create_account(&form.email, &form.password) {
        Ok(c) => c,
        // create_account's errors are all user-facing input problems
        // (bad email, weak password, duplicate) — surface them as-is.
        Err(e) => return signup_error(&e.to_string()),
    };

    // The account exists. Stash the authenticated login and show the
    // recovery code once; the user resumes the flow via /continue.
    let cont = Continuation {
        client_id: f.client_id.clone(),
        profile_ulid: created.profile_ulid.clone(),
        email: created.email.clone(),
        redirect_uri: f.redirect_uri.clone(),
        nonce: f.nonce().map(str::to_string),
        code_challenge: f.code_challenge.clone(),
        scope: f.scope.clone(),
        state: f.state.clone(),
        auth_time: now_unix(),
    };
    let ttl = app.config.session.code_ttl_secs.max(300);
    let token = match app.idp_store.stash_continuation(&cont, ttl) {
        Ok(t) => t,
        Err(e) => return ApiError::from(e).into_response(),
    };
    let continue_url = format!("/continue?token={}", urlencoding::encode(&token));
    Html(html::recovery_page(&continue_url, &created.recovery_code)).into_response()
}

/// `GET /continue` — the recovery-code page's "I saved it" button lands
/// here. Consumes the continuation token and resumes the authorize flow:
/// mints a code and redirects to the RP.
#[derive(Debug, Deserialize)]
pub struct ContinueParams {
    pub token: String,
}

pub async fn continue_flow(
    State(app): State<AppState>,
    Query(q): Query<ContinueParams>,
) -> Response {
    let cont = match app.idp_store.take_continuation(&q.token) {
        Ok(Some(c)) => c,
        Ok(None) => {
            return (
                StatusCode::BAD_REQUEST,
                Html(html::error_page(
                    "This sign-up link has expired. Start again from the application.",
                )),
            )
                .into_response()
        }
        Err(e) => return ApiError::from(e).into_response(),
    };

    let code = match app.idp_store.issue_code(
        &cont.client_id,
        &cont.profile_ulid,
        &cont.email,
        &cont.redirect_uri,
        cont.nonce.as_deref(),
        &cont.code_challenge,
        &cont.scope,
        app.config.session.code_ttl_secs,
    ) {
        Ok(c) => c,
        Err(e) => return ApiError::from(e).into_response(),
    };
    redirect_to_rp(&cont.redirect_uri, &code, &cont.state)
}

/// Mint an authorization code for an authenticated user and 302 the
/// browser back to the RP's `redirect_uri?code=…&state=…`.
fn issue_and_redirect(
    app: &AppState,
    f: &FlowFields,
    profile_ulid: &str,
    email: &str,
    _auth_time: i64,
) -> ApiResult<Response> {
    let code = app
        .idp_store
        .issue_code(
            &f.client_id,
            profile_ulid,
            email,
            &f.redirect_uri,
            f.nonce(),
            &f.code_challenge,
            &f.scope,
            app.config.session.code_ttl_secs,
        )
        .map_err(ApiError::from)?;
    Ok(redirect_to_rp(&f.redirect_uri, &code, &f.state))
}

/// Build the 302 back to the RP carrying `code` + `state`.
fn redirect_to_rp(redirect_uri: &str, code: &str, state: &str) -> Response {
    let sep = if redirect_uri.contains('?') { '&' } else { '?' };
    let location = format!(
        "{redirect_uri}{sep}code={}&state={}",
        urlencoding::encode(code),
        urlencoding::encode(state),
    );
    let mut headers = HeaderMap::new();
    if let Ok(v) = location.parse() {
        headers.insert(LOCATION, v);
    }
    (StatusCode::FOUND, headers).into_response()
}

// --- /token ----------------------------------------------------------------

/// `POST /token` form body — `application/x-www-form-urlencoded`.
#[derive(Debug, Deserialize)]
pub struct TokenForm {
    pub grant_type: Option<String>,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub code_verifier: Option<String>,
}

/// An OAuth 2.0 token-endpoint error — a 400 with the standard
/// `{ "error": ... }` body.
fn token_error(error: &str, description: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
}

/// `POST /token` — exchange an authorization code (+ PKCE verifier) for the
/// token set.
pub async fn token(State(app): State<AppState>, Form(form): Form<TokenForm>) -> Response {
    if form.grant_type.as_deref() != Some("authorization_code") {
        return token_error(
            "unsupported_grant_type",
            "only authorization_code is supported",
        );
    }
    let code = match form.code.as_deref().filter(|s| !s.is_empty()) {
        Some(c) => c,
        None => return token_error("invalid_request", "missing code"),
    };
    let client_id = match form.client_id.as_deref().filter(|s| !s.is_empty()) {
        Some(c) => c,
        None => return token_error("invalid_request", "missing client_id"),
    };
    let client = match app.clients.get(client_id) {
        Some(c) => c,
        None => return token_error("invalid_client", "unknown client_id"),
    };

    // Confidential clients authenticate by client_secret; public clients
    // by PKCE alone.
    if !client.public {
        let presented = form.client_secret.as_deref().unwrap_or("");
        if client.client_secret.is_empty() || presented != client.client_secret {
            return token_error("invalid_client", "client authentication failed");
        }
    }

    // Redeem the code: exists, unexpired, unused.
    let redeemed = match app.idp_store.redeem_code(code) {
        Ok(Ok(r)) => r,
        Ok(Err(CodeError::Unknown)) => {
            return token_error("invalid_grant", "authorization code is invalid")
        }
        Ok(Err(CodeError::Expired)) => {
            return token_error("invalid_grant", "authorization code has expired")
        }
        Ok(Err(CodeError::AlreadyUsed)) => {
            return token_error("invalid_grant", "authorization code already used")
        }
        Err(e) => return ApiError::from(e).into_response(),
    };

    // The code is bound to the client + redirect URI it was minted for.
    if redeemed.client_id != client_id {
        return token_error("invalid_grant", "code was issued to a different client");
    }
    match form.redirect_uri.as_deref() {
        Some(r) if r == redeemed.redirect_uri => {}
        _ => return token_error("invalid_grant", "redirect_uri mismatch"),
    }

    // PKCE: the presented verifier must transform to the stored challenge.
    let verifier = match form.code_verifier.as_deref().filter(|s| !s.is_empty()) {
        Some(v) => v,
        None => return token_error("invalid_request", "missing code_verifier"),
    };
    if !verify_pkce_s256(verifier, &redeemed.code_challenge) {
        return token_error("invalid_grant", "PKCE verification failed");
    }

    // All checks passed — mint the token set.
    let id_token = match mint_id_token(
        &app.signing_key,
        &app.config.server.issuer,
        &redeemed.profile_ulid,
        &redeemed.client_id,
        &redeemed.email,
        false, // email_verified — see IdTokenClaims
        redeemed.nonce.as_deref(),
        redeemed.auth_time,
    ) {
        Ok(t) => t,
        Err(e) => return ApiError::from(e).into_response(),
    };

    let access_token = match app.idp_store.issue_access_token(
        &redeemed.profile_ulid,
        &redeemed.email,
        &redeemed.scope,
        ACCESS_TOKEN_TTL_SECS,
    ) {
        Ok(t) => t,
        Err(e) => return ApiError::from(e).into_response(),
    };
    // The refresh token is opaque + stored like an access token, with a
    // longer life. Phase 2 issues it; the refresh grant itself is future
    // work — RPs that re-run the authorize flow do not need it yet.
    let refresh_token = match app.idp_store.issue_access_token(
        &redeemed.profile_ulid,
        &redeemed.email,
        &redeemed.scope,
        ACCESS_TOKEN_TTL_SECS * 24 * 30,
    ) {
        Ok(t) => t,
        Err(e) => return ApiError::from(e).into_response(),
    };

    Json(json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": ACCESS_TOKEN_TTL_SECS,
        "refresh_token": refresh_token,
        "id_token": id_token,
        "scope": redeemed.scope,
    }))
    .into_response()
}

// --- /userinfo -------------------------------------------------------------

/// `GET /userinfo` — resolve a bearer access token to the user's claims.
pub async fn userinfo(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let token = match bearer_token(&headers) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                [("WWW-Authenticate", "Bearer")],
                Json(json!({ "error": "invalid_token" })),
            )
                .into_response()
        }
    };
    match app.idp_store.lookup_access_token(&token) {
        Ok(Some(id)) => Json(json!({
            "sub": id.profile_ulid,
            "email": id.email,
            "email_verified": false,
        }))
        .into_response(),
        Ok(None) => (
            StatusCode::UNAUTHORIZED,
            [("WWW-Authenticate", "Bearer error=\"invalid_token\"")],
            Json(json!({ "error": "invalid_token" })),
        )
            .into_response(),
        Err(e) => ApiError::from(e).into_response(),
    }
}

/// Extract a `Bearer` token from the `Authorization` header.
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let token = raw.strip_prefix("Bearer ").or_else(|| raw.strip_prefix("bearer "))?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
