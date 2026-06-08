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
use crate::store::{canonical_recovery, sha256_hex};
use crate::token::{mint_id_token, ACCESS_TOKEN_TTL_SECS};
use axum::extract::{Form, Query, State};
use axum::http::header::{HeaderMap, AUTHORIZATION, COOKIE, LOCATION, SET_COOKIE};
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

// --- SSO session cookie ----------------------------------------------------

/// The IdP SSO session cookie name.
const SSO_COOKIE: &str = "ohd_idp_sso";

/// Read the SSO session token from the request `Cookie` header, if present.
fn sso_cookie(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        let pair = pair.trim();
        if let Some(v) = pair.strip_prefix(&format!("{SSO_COOKIE}=")) {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Build the `Set-Cookie` value that plants the SSO session.
/// `Secure`, `HttpOnly`, `SameSite=Lax`, scoped to the whole site, with a
/// `Max-Age` matching the session TTL.
fn set_sso_cookie(token: &str, ttl_secs: i64) -> String {
    format!(
        "{SSO_COOKIE}={token}; Path=/; Max-Age={ttl_secs}; \
         HttpOnly; Secure; SameSite=Lax"
    )
}

/// Build the `Set-Cookie` value that clears the SSO session — an empty
/// value with `Max-Age=0` so the browser drops it immediately.
fn clear_sso_cookie() -> String {
    format!("{SSO_COOKIE}=; Path=/; Max-Age=0; HttpOnly; Secure; SameSite=Lax")
}

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

/// `GET /authorize` — the OIDC authorization endpoint.
///
/// - **SSO hit:** a valid, unexpired SSO cookie → skip the login page,
///   resolve the profile from the session, and go straight to minting an
///   authorization code + redirecting to the RP. A second RP login is
///   therefore promptless.
/// - **SSO miss:** no cookie, or an expired/unknown session → render the
///   login page.
pub async fn authorize(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(p): Query<AuthorizeParams>,
) -> Response {
    let v = match validate_authorize(&app, &p) {
        Ok(v) => v,
        Err(msg) => {
            return (StatusCode::BAD_REQUEST, Html(html::error_page(&msg))).into_response()
        }
    };

    // SSO hit: a live session cookie short-circuits straight to a code.
    if let Some(token) = sso_cookie(&headers) {
        match app.idp_store.lookup_session(&token) {
            Ok(Some(session)) => {
                let f = FlowFields {
                    client_id: v.client_id.clone(),
                    redirect_uri: v.redirect_uri.clone(),
                    scope: v.scope.clone(),
                    state: v.state.clone(),
                    nonce: v.nonce.clone(),
                    code_challenge: v.code_challenge.clone(),
                };
                return match issue_and_redirect(
                    &app,
                    &f,
                    &session.profile_ulid,
                    &session.email,
                    session.auth_time,
                    None, // the cookie already exists — do not re-set it
                ) {
                    Ok(resp) => resp,
                    Err(e) => e.into_response(),
                };
            }
            // Unknown / expired session → fall through to the login page.
            Ok(None) => {}
            Err(e) => return ApiError::from(e).into_response(),
        }
    }

    // SSO miss: render the login page.
    Html(html::login_page(
        &v.client_id,
        &v.redirect_uri,
        &v.scope,
        &v.state,
        v.nonce.as_deref(),
        &v.code_challenge,
        app.config.signup.open,
        app.config.recovery.enabled,
        None,
    ))
    .into_response()
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
        app.config.recovery.enabled,
        None,
    ))
    .into_response()
}

/// `POST /login` form body — the flow fields plus the credentials. The
/// page submits one of two credential shapes; both fields are optional so
/// the form deserializes either way:
///
/// - **password login:** `email` + `password`.
/// - **recovery-code login:** `recovery_code` (gated on
///   `config.recovery.enabled`).
#[derive(Debug, Deserialize)]
pub struct LoginForm {
    #[serde(flatten)]
    pub flow: FlowFields,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub recovery_code: Option<String>,
}

/// `POST /login` — verify the credentials, then continue the authorize
/// flow by minting a code and redirecting. On failure re-render `/login`
/// with an error banner. Handles both the password and recovery-code
/// branches.
pub async fn login_submit(
    State(app): State<AppState>,
    Form(form): Form<LoginForm>,
) -> Response {
    let f = &form.flow;
    if let Err(msg) = f.revalidate(&app) {
        return (StatusCode::BAD_REQUEST, Html(html::error_page(&msg))).into_response();
    }

    // Recovery-code branch — taken when a non-empty recovery code is
    // submitted and recovery login is enabled.
    let recovery = form
        .recovery_code
        .as_deref()
        .filter(|s| !s.trim().is_empty());
    if let Some(code) = recovery {
        if !app.config.recovery.enabled {
            return login_error(&app, f, "Recovery-code sign-in is disabled.");
        }
        let hash = sha256_hex(&canonical_recovery(code));
        let account = match app.accounts.find_by_recovery_hash(&hash) {
            Ok(Some(a)) => a,
            Ok(None) => return login_error(&app, f, "That recovery code is not valid."),
            Err(e) => return ApiError::from(e).into_response(),
        };
        // The recovery code resolves the profile directly. The email may
        // be absent (a profile with no email credential yet); the
        // `id_token` carries an empty string in that case — honest about
        // what the IdP knows.
        let email = account.email.unwrap_or_default();
        let auth_time = now_unix();
        return finish_login(&app, f, &account.profile_ulid, &email, auth_time);
    }

    // Password branch.
    let email = match form.email.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(e) => e,
        None => return login_error(&app, f, "Enter your email and password."),
    };
    let password = form.password.as_deref().unwrap_or("");
    let account = match app.accounts.find_by_email(email) {
        Ok(Some(a)) => a,
        Ok(None) => return login_error(&app, f, "Incorrect email or password."),
        Err(e) => return ApiError::from(e).into_response(),
    };
    if !account.verify_password(password) {
        return login_error(&app, f, "Incorrect email or password.");
    }

    let auth_time = now_unix();
    finish_login(&app, f, &account.profile_ulid, &account.email, auth_time)
}

/// Complete an authenticated login: mint a bounded SSO session, then mint
/// an authorization code and redirect to the RP carrying both the `code`
/// and the SSO `Set-Cookie`.
fn finish_login(
    app: &AppState,
    f: &FlowFields,
    profile_ulid: &str,
    email: &str,
    auth_time: i64,
) -> Response {
    let ttl = sso_ttl_secs(app);
    let cookie = match app
        .idp_store
        .create_session(profile_ulid, email, auth_time, ttl)
    {
        Ok(token) => Some(set_sso_cookie(&token, ttl)),
        Err(e) => return ApiError::from(e).into_response(),
    };
    match issue_and_redirect(app, f, profile_ulid, email, auth_time, cookie) {
        Ok(resp) => resp,
        Err(e) => e.into_response(),
    }
}

/// The SSO session lifetime in seconds, derived from
/// `config.session.sso_ttl_hours` (floored at one hour).
fn sso_ttl_secs(app: &AppState) -> i64 {
    app.config.session.sso_ttl_hours.max(1) * 3600
}

/// Re-render the login page with a credential error. Callers pass a
/// message that does not enable account enumeration — the same wording for
/// "no such email" and "wrong password".
fn login_error(app: &AppState, f: &FlowFields, message: &str) -> Response {
    Html(html::login_page(
        &f.client_id,
        &f.redirect_uri,
        &f.scope,
        &f.state,
        f.nonce(),
        &f.code_challenge,
        app.config.signup.open,
        app.config.recovery.enabled,
        Some(message),
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
        // create_account returns an anyhow::Error. `{:#}` prints the
        // full context chain (each `.context("…")` layer joined by ": "
        // plus the root cause), so an opaque "inserting profiles row"
        // turns into "inserting profiles row: attempt to write a
        // readonly database" — the operator can act on what they see
        // without having to ssh in and tail logs.
        Err(e) => return signup_error(&format!("{e:#}")),
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
    // The token rides as a hidden form field — recovery_page builds a GET
    // form to /continue, and a GET form ignores any query on its action URL.
    Html(html::recovery_page(&token, &created.recovery_code)).into_response()
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

    // The sign-up just authenticated the user — mint the SSO session here
    // so a subsequent RP login is promptless, the same as a password login.
    let ttl = sso_ttl_secs(&app);
    let cookie = match app.idp_store.create_session(
        &cont.profile_ulid,
        &cont.email,
        cont.auth_time,
        ttl,
    ) {
        Ok(token) => Some(set_sso_cookie(&token, ttl)),
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
    redirect_to_rp(&cont.redirect_uri, &code, &cont.state, cookie)
}

/// Mint an authorization code for an authenticated user and 302 the
/// browser back to the RP's `redirect_uri?code=…&state=…`. An optional
/// `Set-Cookie` value plants the SSO session on the same response.
fn issue_and_redirect(
    app: &AppState,
    f: &FlowFields,
    profile_ulid: &str,
    email: &str,
    _auth_time: i64,
    set_cookie: Option<String>,
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
    Ok(redirect_to_rp(&f.redirect_uri, &code, &f.state, set_cookie))
}

/// Build the 302 back to the RP carrying `code` + `state`, optionally with
/// a `Set-Cookie` header for the SSO session.
fn redirect_to_rp(
    redirect_uri: &str,
    code: &str,
    state: &str,
    set_cookie: Option<String>,
) -> Response {
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
    if let Some(cookie) = set_cookie {
        if let Ok(v) = cookie.parse() {
            headers.insert(SET_COOKIE, v);
        }
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
        app.keys.active(),
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

// --- /reset (password reset via recovery code) -----------------------------

/// `GET /reset` — the "forgot password?" page: enter a recovery code + a
/// new password. Reachable from the login page's link, carrying the
/// authorize-flow params so a successful reset resumes the original flow.
pub async fn reset_form(State(app): State<AppState>, Query(f): Query<FlowFields>) -> Response {
    if !app.config.recovery.enabled {
        return (
            StatusCode::FORBIDDEN,
            Html(html::error_page("Recovery-code password reset is disabled.")),
        )
            .into_response();
    }
    if let Err(msg) = f.revalidate(&app) {
        return (StatusCode::BAD_REQUEST, Html(html::error_page(&msg))).into_response();
    }
    Html(html::reset_page(
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

/// `POST /reset` form body — flow fields + the recovery code + the new
/// password. `email` is only needed when the profile has no email
/// credential yet (see [`crate::store::AccountStore::reset_password`]).
#[derive(Debug, Deserialize)]
pub struct ResetForm {
    #[serde(flatten)]
    pub flow: FlowFields,
    pub recovery_code: String,
    pub password: String,
    pub confirm: String,
    #[serde(default)]
    pub email: Option<String>,
}

/// `POST /reset` — validate the recovery code, set the new password, then
/// continue the authorize flow exactly as a fresh login does (mint an SSO
/// session + an authorization code, redirect to the RP).
pub async fn reset_submit(State(app): State<AppState>, Form(form): Form<ResetForm>) -> Response {
    if !app.config.recovery.enabled {
        return (
            StatusCode::FORBIDDEN,
            Html(html::error_page("Recovery-code password reset is disabled.")),
        )
            .into_response();
    }
    let f = &form.flow;
    if let Err(msg) = f.revalidate(&app) {
        return (StatusCode::BAD_REQUEST, Html(html::error_page(&msg))).into_response();
    }

    let reset_error = |msg: &str| -> Response {
        Html(html::reset_page(
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
        return reset_error("The two passwords do not match.");
    }

    // Resolve the profile from the recovery code.
    let hash = sha256_hex(&canonical_recovery(&form.recovery_code));
    let account = match app.accounts.find_by_recovery_hash(&hash) {
        Ok(Some(a)) => a,
        Ok(None) => return reset_error("That recovery code is not valid."),
        Err(e) => return ApiError::from(e).into_response(),
    };

    // Set (or create) the password. A profile with no email credential
    // yet needs an email supplied to create one.
    let new_email = form.email.as_deref().filter(|s| !s.trim().is_empty());
    let email = match app
        .accounts
        .reset_password(&account.profile_ulid, &form.password, new_email)
    {
        Ok(email) => email,
        // reset_password's errors are user-facing input problems
        // (no email given, weak password, email already taken).
        Err(e) => return reset_error(&e.to_string()),
    };

    // The recovery code authenticated the user — continue the flow with a
    // fresh SSO session, just like a password login.
    let auth_time = now_unix();
    finish_login(&app, f, &account.profile_ulid, &email, auth_time)
}

// --- /logout (RP-Initiated Logout) -----------------------------------------

/// `GET`/`POST /logout` query/form parameters.
#[derive(Debug, Deserialize)]
pub struct LogoutParams {
    /// Where to send the browser after logout — only honoured if it
    /// exactly matches a registered client redirect URI.
    #[serde(default)]
    pub post_logout_redirect_uri: Option<String>,
    /// Echoed back on the post-logout redirect, per RP-Initiated Logout.
    #[serde(default)]
    pub state: Option<String>,
}

/// `GET`/`POST /logout` — RP-Initiated Logout. Deletes the IdP SSO session
/// and clears the cookie. If `post_logout_redirect_uri` is supplied and
/// exactly matches a registered client redirect URI the browser is
/// redirected there; otherwise a plain "signed out" page is shown.
pub async fn logout(
    State(app): State<AppState>,
    headers: HeaderMap,
    params: Option<Query<LogoutParams>>,
) -> Response {
    let params = params.map(|Query(p)| p).unwrap_or(LogoutParams {
        post_logout_redirect_uri: None,
        state: None,
    });

    // Delete the session server-side, if the browser carried one.
    if let Some(token) = sso_cookie(&headers) {
        if let Err(e) = app.idp_store.delete_session(&token) {
            return ApiError::from(e).into_response();
        }
    }

    let clear = clear_sso_cookie();

    // A post-logout redirect is only honoured when it exactly matches a
    // registered redirect URI — never an open redirect.
    if let Some(uri) = params
        .post_logout_redirect_uri
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        let registered = app
            .config
            .clients
            .iter()
            .any(|c| c.redirect_uris.iter().any(|u| u == uri));
        if registered {
            let location = match &params.state {
                Some(s) if !s.is_empty() => {
                    let sep = if uri.contains('?') { '&' } else { '?' };
                    format!("{uri}{sep}state={}", urlencoding::encode(s))
                }
                _ => uri.to_string(),
            };
            let mut hdrs = HeaderMap::new();
            if let Ok(v) = location.parse() {
                hdrs.insert(LOCATION, v);
            }
            if let Ok(v) = clear.parse() {
                hdrs.insert(SET_COOKIE, v);
            }
            return (StatusCode::FOUND, hdrs).into_response();
        }
        // An unregistered redirect target is silently ignored — fall
        // through to the plain signed-out page.
    }

    // No (valid) redirect target — show the signed-out page, still
    // clearing the cookie.
    let mut hdrs = HeaderMap::new();
    if let Ok(v) = clear.parse() {
        hdrs.insert(SET_COOKIE, v);
    }
    (StatusCode::OK, hdrs, Html(html::logged_out_page())).into_response()
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
