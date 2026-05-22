//! Optional OAuth 2.0 + OpenID Connect IdP endpoints for the storage server.
//!
//! When the storage daemon is launched with `--oauth-issuer <URL>` it lights
//! up a sibling axum sub-router at `/oauth/*` + `/.well-known/openid-configuration`
//! turning the storage instance into a self-contained OIDC issuer. This is
//! opt-in: most deployments delegate to external Google / Okta / Authentik /
//! etc. The self-IdP path is for self-hosted users + offline-first scenarios
//! where running an external IdP isn't practical.
//!
//! # Endpoints
//!
//! | Path | Spec | Purpose |
//! |---|---|---|
//! | `GET /.well-known/openid-configuration` | OIDC Discovery 1.0 | Discovery JSON |
//! | `GET /oauth/jwks.json` | RFC 7517 | Public JWK Set we mint id_tokens with |
//! | `GET /oauth/authorize` | RFC 6749 §4.1 + RFC 7636 | Authorization-code + PKCE start |
//! | `POST /oauth/token` | RFC 6749 §4.1.3 / §6 / §4.4 / RFC 8628 §3.4 | Token exchange |
//! | `POST /oauth/device` | RFC 8628 §3.1 | Device-code start |
//! | `GET /oauth/device-confirm` + `POST /oauth/device-confirm` | RFC 8628 §3.3 | User confirms a user_code |
//! | `GET /oauth/userinfo` | OIDC Core §5.3 | UserInfo (sub, identities) |
//! | `POST /oauth/register` | RFC 7591 | Dynamic Client Registration |
//!
//! # Login model (v0)
//!
//! The HTML pages for `/oauth/authorize` and `/oauth/device-confirm` are
//! intentionally minimal: a `<textarea>` that accepts an existing self-session
//! token (`ohds_…`). Users acquire that token out-of-band — typically via the
//! `issue-self-token` CLI subcommand on the same machine as the storage,
//! optionally with a multi-identity flow that links an external OIDC `(iss,
//! sub)` to the same `user_ulid`. The storage daemon never asks for a
//! password directly. Treating the self-session token as the "user
//! credential" the AS sees keeps the v0 storage IdP narrow + auditable;
//! richer login UX (email / password, WebAuthn) is the deliverable that
//! follows when consumer apps want a fully-self-contained sign-in box.
//!
//! # Schema bootstrap
//!
//! The `oauth_*` tables live in the same per-user file. We can't add a
//! migration to `format.rs` from the server crate (core is owned by another
//! agent), so we run an idempotent `CREATE TABLE IF NOT EXISTS` block on the
//! first call to [`bootstrap`]. The DDL is identical to
//! `migrations/012_oauth_state.sql`. When migration 012 lands properly in
//! `format.rs`, the bootstrap call becomes a no-op.

use std::sync::Arc;

use axum::extract::{Form, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use ohd_storage_core::auth as ohd_auth;
use ohd_storage_core::storage::Storage;
use rusqlite::params;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

// Use explicit `#[path]` so submodule resolution works in two contexts:
//
//   1. Normal lib/bin: `src/oauth.rs` finds `src/oauth/schema.rs` via the
//      default rule (a non-mod-rs file's children live in a folder named
//      after the file).
//   2. Integration tests that pull `src/oauth.rs` in via
//      `#[path = "../src/oauth.rs"] mod oauth;` — there the submodule
//      directory rule is computed from the *test file's* directory, so the
//      default search would look at `tests/oauth/schema.rs` and miss.
//      Spelling out the path makes both invocations point at the same files.
#[path = "oauth/providers.rs"]
pub mod providers;
#[path = "oauth/schema.rs"]
pub mod schema;
#[path = "oauth/signing.rs"]
pub mod signing;

pub use providers::OidcProvider;
pub use schema::bootstrap;

use ohd_storage_core::identities as ohd_identities;

/// Default authorization-code TTL (seconds). One minute.
const AUTH_CODE_TTL_S: i64 = 60;
/// Default access-token TTL (seconds). One hour.
const ACCESS_TOKEN_TTL_S: i64 = 3600;
/// Default refresh-token TTL (seconds). Thirty days.
const REFRESH_TOKEN_TTL_S: i64 = 30 * 86_400;
/// Default device-code TTL (seconds). Ten minutes.
const DEVICE_CODE_TTL_S: i64 = 600;
/// Default device-code poll interval (seconds).
const DEVICE_CODE_POLL_INTERVAL_S: i64 = 5;
/// TTL for an in-progress OIDC-RP login (`oauth_pending_logins`). Ten minutes
/// — covers the user's round-trip through the upstream provider's login page.
const PENDING_LOGIN_TTL_S: i64 = 600;

/// Live config + handle bundle owned by the axum sub-router.
#[derive(Clone)]
pub struct OauthState {
    pub storage: Arc<Storage>,
    /// Issuer URL. Configurable via `--oauth-issuer`. Used as `iss` in
    /// id_tokens and as the discovery doc base URL.
    pub issuer: String,
    /// Upstream OIDC providers the AS can delegate login to (the RP side).
    /// Empty => no provider login; only the paste-a-self-session-token UX is
    /// offered (on-device / self-hosted mode). Server/cloud storage configures
    /// at least `ohd_account`. See [`providers`].
    pub providers: Arc<Vec<OidcProvider>>,
}

impl OauthState {
    fn provider(&self, key: &str) -> Option<&OidcProvider> {
        self.providers.iter().find(|p| p.key == key)
    }
}

/// Build the axum Router that serves the OIDC + OAuth endpoints. Mount at
/// the deployment's HTTP root; the routes themselves namespace under
/// `/oauth/*` and `/.well-known/openid-configuration`.
pub fn router(state: OauthState) -> Router {
    Router::new()
        .route("/.well-known/openid-configuration", get(discovery_handler))
        // Some clients (Auth0-style metadata) look here too. Keep both alive
        // — both serve the same payload; the OIDC variant is the "official"
        // OpenID one and the OAuth variant is RFC 8414. They overlap heavily.
        .route(
            "/.well-known/oauth-authorization-server",
            get(discovery_handler),
        )
        .route("/oauth/jwks.json", get(jwks_handler))
        .route("/oauth/authorize", get(authorize_get).post(authorize_post))
        // OIDC-RP callback: where a configured upstream provider
        // (`accounts.ohd.dev` etc.) redirects the user back after they
        // authenticate. Storage verifies the provider's id_token here,
        // resolves/creates the storage user, then resumes its own AS code
        // issuance back to the OHD client.
        .route("/oauth/oidc-callback", get(oidc_callback_handler))
        .route("/oauth/token", post(token_handler))
        .route("/oauth/device", post(device_authorize_handler))
        .route(
            "/oauth/device-confirm",
            get(device_confirm_get).post(device_confirm_post),
        )
        .route(
            "/oauth/userinfo",
            get(userinfo_handler).post(userinfo_handler),
        )
        .route("/oauth/register", post(register_handler))
        .with_state(state)
}

// ============================================================================
// Discovery (.well-known/openid-configuration)
// ============================================================================

#[derive(Serialize)]
struct DiscoveryDoc<'a> {
    issuer: &'a str,
    authorization_endpoint: String,
    token_endpoint: String,
    device_authorization_endpoint: String,
    jwks_uri: String,
    userinfo_endpoint: String,
    registration_endpoint: String,
    response_types_supported: &'a [&'a str],
    grant_types_supported: &'a [&'a str],
    subject_types_supported: &'a [&'a str],
    id_token_signing_alg_values_supported: &'a [&'a str],
    code_challenge_methods_supported: &'a [&'a str],
    token_endpoint_auth_methods_supported: &'a [&'a str],
    scopes_supported: &'a [&'a str],
    claims_supported: &'a [&'a str],
}

async fn discovery_handler(State(state): State<OauthState>) -> Response {
    let issuer = state.issuer.trim_end_matches('/').to_string();
    let doc = DiscoveryDoc {
        issuer: &issuer,
        authorization_endpoint: format!("{issuer}/oauth/authorize"),
        token_endpoint: format!("{issuer}/oauth/token"),
        device_authorization_endpoint: format!("{issuer}/oauth/device"),
        jwks_uri: format!("{issuer}/oauth/jwks.json"),
        userinfo_endpoint: format!("{issuer}/oauth/userinfo"),
        registration_endpoint: format!("{issuer}/oauth/register"),
        response_types_supported: &["code"],
        grant_types_supported: &[
            "authorization_code",
            "refresh_token",
            "urn:ietf:params:oauth:grant-type:device_code",
        ],
        subject_types_supported: &["public"],
        id_token_signing_alg_values_supported: &["RS256"],
        code_challenge_methods_supported: &["S256"],
        token_endpoint_auth_methods_supported: &["none", "client_secret_post"],
        scopes_supported: &["openid", "profile", "offline_access"],
        claims_supported: &["sub", "iss", "aud", "exp", "iat", "auth_time"],
    };
    (StatusCode::OK, Json(doc)).into_response()
}

// ============================================================================
// /oauth/jwks.json
// ============================================================================

async fn jwks_handler(State(state): State<OauthState>) -> Response {
    match signing::list_active_jwks(&state.storage) {
        Ok(set) => (StatusCode::OK, Json(set)).into_response(),
        Err(e) => oauth_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        ),
    }
}

// ============================================================================
// /oauth/authorize  — Authorization Code + PKCE start
// ============================================================================

#[derive(Deserialize)]
struct AuthorizeQuery {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    scope: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    /// The OHD client's OIDC `nonce` — must be echoed into the issued
    /// id_token's `nonce` claim or an OIDC relying party rejects the token.
    nonce: Option<String>,
    /// Optional catalog key of a provider to use directly. When supplied (and
    /// valid) the AS skips its provider-picker page and 302s straight to that
    /// upstream provider — lets an OHD client deep-link "Sign in with OHD".
    provider: Option<String>,
}

async fn authorize_get(
    State(state): State<OauthState>,
    Query(q): Query<AuthorizeQuery>,
) -> Response {
    if let Err(resp) = validate_authorize_params(&state, &q) {
        return resp;
    }
    // Provider deep-link: a client can pass `?provider=ohd_account` to skip
    // the picker and go straight to "Sign in with OHD".
    if let Some(provider_key) = q.provider.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return begin_oidc_login(&state, &q, provider_key).await;
    }
    let html = render_authorize_form(&state, &q);
    (StatusCode::OK, Html(html)).into_response()
}

#[derive(Deserialize)]
struct AuthorizePost {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    scope: Option<String>,
    state: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    /// The OHD client's OIDC `nonce` — echoed into the issued id_token.
    nonce: Option<String>,
    /// User-pasted self-session token (the on-device / fast-path login UX).
    self_session_token: Option<String>,
    /// Catalog key of a configured upstream OIDC provider the user picked
    /// (e.g. `ohd_account`). When present the AS runs the OIDC-RP flow.
    provider: Option<String>,
}

async fn authorize_post(State(state): State<OauthState>, Form(f): Form<AuthorizePost>) -> Response {
    let q = AuthorizeQuery {
        response_type: f.response_type,
        client_id: f.client_id,
        redirect_uri: f.redirect_uri,
        scope: f.scope,
        state: f.state,
        code_challenge: f.code_challenge,
        code_challenge_method: f.code_challenge_method,
        nonce: f.nonce,
        provider: f.provider.clone(),
    };
    if let Err(resp) = validate_authorize_params(&state, &q) {
        return resp;
    }

    // Branch 1: the user picked an upstream OIDC provider ("Sign in with OHD").
    if let Some(provider_key) = f.provider.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        return begin_oidc_login(&state, &q, provider_key).await;
    }

    // Branch 2: the on-device fast path — a pasted self-session token.
    let bearer = match f.self_session_token.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "access_denied",
                "pick a sign-in provider or paste a self-session token",
            );
        }
    };
    let resolved = match state
        .storage
        .with_conn(|conn| ohd_auth::resolve_token(conn, &bearer))
    {
        Ok(r) if r.kind == ohd_auth::TokenKind::SelfSession => r,
        Ok(_) => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "access_denied",
                "only self-session tokens are accepted as login credentials",
            );
        }
        Err(e) => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "access_denied",
                &format!("invalid self-session token: {e}"),
            );
        }
    };
    issue_downstream_code(
        &state,
        q.client_id.as_deref().unwrap_or(""),
        resolved.user_ulid,
        q.redirect_uri.as_deref().unwrap_or(""),
        q.scope.as_deref().unwrap_or(""),
        q.code_challenge.as_deref().unwrap_or(""),
        q.code_challenge_method.as_deref().unwrap_or(""),
        q.nonce.as_deref().unwrap_or(""),
        q.state.as_deref().unwrap_or(""),
    )
}

/// Mint an OHD authorization code bound to `user_ulid` and 302 the browser
/// back to the OHD client's `redirect_uri`. Shared by the self-session path
/// and the OIDC-callback path — both end the same way.
#[allow(clippy::too_many_arguments)]
fn issue_downstream_code(
    state: &OauthState,
    client_id: &str,
    user_ulid: [u8; 16],
    redirect_uri: &str,
    scope: &str,
    code_challenge: &str,
    code_challenge_method: &str,
    nonce: &str,
    client_state: &str,
) -> Response {
    let code = mint_random_token();
    let code_hash = sha256_hex(&code);
    let now = ohd_storage_core::format::now_ms();
    let res = state.storage.with_conn(|conn| {
        conn.execute(
            "INSERT INTO oauth_authorization_codes
                (code_hash, client_id, user_ulid, redirect_uri, scope,
                 code_challenge, code_challenge_method, nonce,
                 issued_at_ms, expires_at_ms, used_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL)",
            params![
                code_hash,
                client_id,
                user_ulid.to_vec(),
                redirect_uri,
                scope,
                code_challenge,
                code_challenge_method,
                nonce,
                now,
                now + AUTH_CODE_TTL_S * 1000,
            ],
        )
        .map_err(ohd_storage_core::Error::from)
    });
    if let Err(e) = res {
        return oauth_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        );
    }
    let url = if redirect_uri.contains('?') {
        format!("{redirect_uri}&code={code}&state={client_state}")
    } else {
        format!("{redirect_uri}?code={code}&state={client_state}")
    };
    Redirect::to(&url).into_response()
}

// ============================================================================
// OIDC-RP login — delegate "who are you?" to a configured upstream provider
// ============================================================================

/// Begin an OIDC-RP login: persist the in-progress downstream request, then
/// 302 the user to the chosen provider's `authorization_endpoint`.
async fn begin_oidc_login(state: &OauthState, q: &AuthorizeQuery, provider_key: &str) -> Response {
    let Some(provider) = state.provider(provider_key) else {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            &format!("provider {provider_key:?} is not configured on this storage instance"),
        );
    };
    // Discover the provider's authorization + token endpoints.
    let disc = match discover_provider(provider).await {
        Ok(d) => d,
        Err(e) => {
            return oauth_error_response(
                StatusCode::BAD_GATEWAY,
                "temporarily_unavailable",
                &format!("provider discovery failed: {e}"),
            )
        }
    };
    // Our (RP-side) PKCE pair + state + nonce toward the upstream provider.
    let oidc_state = mint_random_token();
    let oidc_nonce = mint_random_token();
    let pkce_verifier = mint_random_token();
    let pkce_challenge = pkce_s256(&pkce_verifier);
    let now = ohd_storage_core::format::now_ms();
    let store_res = state.storage.with_conn(|conn| {
        conn.execute(
            "INSERT INTO oauth_pending_logins
                (oidc_state, oidc_nonce, pkce_verifier, provider_key,
                 client_id, redirect_uri, scope, client_state,
                 code_challenge, code_challenge_method, client_nonce,
                 issued_at_ms, expires_at_ms, used_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, NULL)",
            params![
                oidc_state,
                oidc_nonce,
                pkce_verifier,
                provider.key,
                q.client_id.as_deref().unwrap_or(""),
                q.redirect_uri.as_deref().unwrap_or(""),
                q.scope.as_deref().unwrap_or(""),
                q.state.as_deref().unwrap_or(""),
                q.code_challenge.as_deref().unwrap_or(""),
                q.code_challenge_method.as_deref().unwrap_or(""),
                q.nonce.as_deref().unwrap_or(""),
                now,
                now + PENDING_LOGIN_TTL_S * 1000,
            ],
        )
        .map_err(ohd_storage_core::Error::from)
    });
    if let Err(e) = store_res {
        return oauth_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        );
    }
    // Our RP redirect_uri — where the provider sends the user back.
    let our_callback = format!(
        "{}/oauth/oidc-callback",
        state.issuer.trim_end_matches('/')
    );
    let mut url = url_with_query(
        &disc.authorization_endpoint,
        &[
            ("response_type", "code"),
            ("client_id", &provider.client_id),
            ("redirect_uri", &our_callback),
            ("scope", &provider.scopes),
            ("state", &oidc_state),
            ("nonce", &oidc_nonce),
            ("code_challenge", &pkce_challenge),
            ("code_challenge_method", "S256"),
        ],
    );
    // Defensive: some providers reject an empty scope.
    if provider.scopes.is_empty() {
        url = url_with_query(
            &disc.authorization_endpoint,
            &[
                ("response_type", "code"),
                ("client_id", &provider.client_id),
                ("redirect_uri", &our_callback),
                ("scope", "openid"),
                ("state", &oidc_state),
                ("nonce", &oidc_nonce),
                ("code_challenge", &pkce_challenge),
                ("code_challenge_method", "S256"),
            ],
        );
    }
    Redirect::to(&url).into_response()
}

#[derive(Deserialize)]
struct OidcCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// OIDC-RP callback. The provider redirected the user here with `?code&state`.
/// We: look up the pending login by `state`, exchange the code at the
/// provider's token endpoint, verify the returned id_token, resolve (or
/// create) the storage user keyed on the id_token `sub`, then resume the
/// downstream AS code issuance back to the OHD client.
async fn oidc_callback_handler(
    State(state): State<OauthState>,
    Query(cb): Query<OidcCallbackQuery>,
) -> Response {
    if let Some(err) = cb.error.as_deref() {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "access_denied",
            &format!(
                "upstream provider returned error {err:?}: {}",
                cb.error_description.as_deref().unwrap_or("")
            ),
        );
    }
    let oidc_state = match cb.state.as_deref().filter(|s| !s.is_empty()) {
        Some(s) => s.to_string(),
        None => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "callback missing state",
            )
        }
    };
    let provider_code = match cb.code.as_deref().filter(|s| !s.is_empty()) {
        Some(c) => c.to_string(),
        None => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "callback missing code",
            )
        }
    };

    // Load + consume the pending login (single-use; CSRF-bound by `state`).
    type PendingRow = (
        i64,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        i64,
        Option<i64>,
    );
    let row: Result<Option<PendingRow>, _> = state.storage.with_conn(|conn| {
        conn.query_row(
            "SELECT id, oidc_nonce, pkce_verifier, provider_key, client_id,
                    redirect_uri, scope, client_state, code_challenge,
                    client_nonce, expires_at_ms, used_at_ms
               FROM oauth_pending_logins WHERE oidc_state = ?1",
            params![oidc_state],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                    r.get(9)?,
                    r.get(10)?,
                    r.get(11)?,
                ))
            },
        )
        .optional()
        .map_err(ohd_storage_core::Error::from)
    });
    let row = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "unknown or stale login state",
            )
        }
        Err(e) => {
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &e.to_string(),
            )
        }
    };
    let (
        pending_id,
        _oidc_nonce,
        pkce_verifier,
        provider_key,
        client_id,
        redirect_uri,
        scope,
        client_state,
        code_challenge,
        client_nonce,
        expires_at_ms,
        used_at_ms,
    ) = row;
    let now = ohd_storage_core::format::now_ms();
    if used_at_ms.is_some() {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "login state already used",
        );
    }
    if expires_at_ms <= now {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "login state expired",
        );
    }
    let mark = state.storage.with_conn(|conn| {
        conn.execute(
            "UPDATE oauth_pending_logins SET used_at_ms = ?1 WHERE id = ?2",
            params![now, pending_id],
        )
        .map_err(ohd_storage_core::Error::from)
    });
    if let Err(e) = mark {
        return oauth_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        );
    }

    let Some(provider) = state.provider(&provider_key) else {
        return oauth_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "provider for this login is no longer configured",
        );
    };
    let disc = match discover_provider(provider).await {
        Ok(d) => d,
        Err(e) => {
            return oauth_error_response(
                StatusCode::BAD_GATEWAY,
                "temporarily_unavailable",
                &format!("provider discovery failed: {e}"),
            )
        }
    };
    let our_callback = format!(
        "{}/oauth/oidc-callback",
        state.issuer.trim_end_matches('/')
    );
    // Exchange the upstream authorization code for the upstream id_token.
    let id_token = match exchange_provider_code(
        provider,
        &disc.token_endpoint,
        &provider_code,
        &our_callback,
        &pkce_verifier,
    )
    .await
    {
        Ok(t) => t,
        Err(e) => {
            return oauth_error_response(
                StatusCode::BAD_GATEWAY,
                "temporarily_unavailable",
                &format!("upstream token exchange failed: {e}"),
            )
        }
    };

    // Verify the upstream id_token (signature against the provider's JWKS,
    // issuer, audience, expiry) — reuses the core RP verifier. The
    // `HttpJwksResolver` builds a blocking reqwest client (which owns its own
    // tokio runtime); constructing/dropping that inside an async task panics,
    // so the whole verify runs on a blocking thread.
    let cfg = ohd_identities::IssuerVerification::new(
        provider.issuer.clone(),
        vec![provider.client_id.clone()],
    );
    let id_token_for_verify = id_token.clone();
    let verify_res = tokio::task::spawn_blocking(move || {
        let jwks = crate::jwks::HttpJwksResolver::new();
        ohd_identities::verify_id_token(&id_token_for_verify, &cfg, &jwks)
    })
    .await;
    let verified = match verify_res {
        Ok(Ok(v)) => v,
        Ok(Err(e)) => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "access_denied",
                &format!("id_token verification failed: {e}"),
            )
        }
        Err(e) => {
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &format!("verify task: {e}"),
            )
        }
    };

    // Resolve (or create) the storage user from the verified `(iss, sub)`.
    let user_ulid = match resolve_or_create_user(&state, provider, &verified) {
        Ok(u) => u,
        Err(e) => {
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &format!("resolve user: {e}"),
            )
        }
    };

    // Resume the downstream AS authorization-code issuance.
    issue_downstream_code(
        &state,
        &client_id,
        user_ulid,
        &redirect_uri,
        &scope,
        &code_challenge,
        "S256",
        &client_nonce,
        &client_state,
    )
}

/// Resolve a verified upstream identity to a storage `user_ulid`.
///
/// - If `(provider, sub)` is already in `_oidc_identities`, return its user.
/// - Otherwise mint a fresh storage user. The `sub` of an `ohd_account`
///   id_token *is* the user's stable OHD `profile_ulid`; if it parses as a
///   ULID we adopt it verbatim so the storage identity matches the OHD
///   identity. For other providers (or an unparseable `sub`) we mint a fresh
///   random ULID. Either way we record the `(provider, sub)` binding via
///   [`ohd_identities::bootstrap_first_identity`].
fn resolve_or_create_user(
    state: &OauthState,
    provider: &OidcProvider,
    verified: &ohd_identities::VerifiedIdToken,
) -> ohd_storage_core::Result<[u8; 16]> {
    // The provider string we key identities under is the issuer URL — the
    // same shape the linking flow (`complete_identity_link`) uses, so a
    // login and a later link converge on one row.
    let provider_str = verified.issuer.as_str();
    if let Some(existing) = state
        .storage
        .with_conn(|conn| ohd_identities::find_user_by_identity(conn, provider_str, &verified.subject))?
    {
        let _ = state.storage.with_conn(|conn| {
            ohd_identities::touch_last_login(conn, provider_str, &verified.subject)
        });
        return Ok(existing);
    }
    // New user. Adopt the `sub` as the user_ulid when it is itself a ULID
    // (the `ohd_account` / profile_ulid case); else mint fresh.
    let user_ulid = if provider.key == "ohd_account" {
        ohd_storage_core::ulid::parse_crockford(&verified.subject)
            .unwrap_or_else(|_| ohd_storage_core::ulid::mint(ohd_storage_core::format::now_ms()))
    } else {
        ohd_storage_core::ulid::mint(ohd_storage_core::format::now_ms())
    };
    state.storage.with_conn(|conn| {
        ohd_identities::bootstrap_first_identity(
            conn,
            user_ulid,
            provider_str,
            &verified.subject,
            verified.email.as_deref(),
            Some(&provider.display_name),
        )
    })?;
    Ok(user_ulid)
}

// ---- Upstream provider HTTP -------------------------------------------------

/// Minimal OIDC discovery doc — the RP side only needs these two endpoints.
#[derive(Deserialize)]
struct ProviderDiscovery {
    authorization_endpoint: String,
    token_endpoint: String,
}

/// Fetch + cache-free discovery for an upstream provider. JWKS is fetched
/// separately by [`crate::jwks::HttpJwksResolver`] during id_token verify.
async fn discover_provider(provider: &OidcProvider) -> Result<ProviderDiscovery, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("build http client: {e}"))?;
    let url = provider.discovery_url();
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("GET {url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("GET {url}: HTTP {}", resp.status()));
    }
    resp.json::<ProviderDiscovery>()
        .await
        .map_err(|e| format!("parse discovery {url}: {e}"))
}

#[derive(Deserialize)]
struct ProviderTokenResponse {
    id_token: Option<String>,
}

/// Exchange an upstream authorization code for the upstream id_token.
async fn exchange_provider_code(
    provider: &OidcProvider,
    token_endpoint: &str,
    code: &str,
    redirect_uri: &str,
    pkce_verifier: &str,
) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("build http client: {e}"))?;
    let mut form: Vec<(&str, &str)> = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", &provider.client_id),
        ("code_verifier", pkce_verifier),
    ];
    if let Some(secret) = provider.client_secret.as_deref() {
        form.push(("client_secret", secret));
    }
    let resp = client
        .post(token_endpoint)
        .form(&form)
        .send()
        .await
        .map_err(|e| format!("POST {token_endpoint}: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("POST {token_endpoint}: HTTP {status}: {body}"));
    }
    let tr: ProviderTokenResponse = resp
        .json()
        .await
        .map_err(|e| format!("parse token response: {e}"))?;
    tr.id_token
        .ok_or_else(|| "upstream token response carried no id_token".to_string())
}

/// Build `base?k1=v1&k2=v2…` with each value percent-encoded.
fn url_with_query(base: &str, params: &[(&str, &str)]) -> String {
    let mut out = String::from(base);
    let sep = if base.contains('?') { '&' } else { '?' };
    let mut first = true;
    for (k, v) in params {
        out.push(if first { sep } else { '&' });
        first = false;
        out.push_str(k);
        out.push('=');
        out.push_str(&percent_encode(v));
    }
    out
}

/// Percent-encode a query-string value (RFC 3986 unreserved set kept literal).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn validate_authorize_params(state: &OauthState, q: &AuthorizeQuery) -> Result<(), Response> {
    if q.response_type.as_deref() != Some("code") {
        return Err(oauth_error_response(
            StatusCode::BAD_REQUEST,
            "unsupported_response_type",
            "only response_type=code is supported",
        ));
    }
    let client_id = match q.client_id.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            return Err(oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "missing client_id",
            ))
        }
    };
    let redirect_uri = match q.redirect_uri.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            return Err(oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "missing redirect_uri",
            ))
        }
    };
    if let Err(e) = require_client_redirect(state, client_id, redirect_uri) {
        return Err(e);
    }
    if q.code_challenge_method.as_deref() != Some("S256") {
        return Err(oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "code_challenge_method must be S256",
        ));
    }
    let cc = q.code_challenge.as_deref().unwrap_or("");
    if cc.len() < 43 {
        return Err(oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "code_challenge missing or too short",
        ));
    }
    Ok(())
}

fn render_authorize_form(state: &OauthState, q: &AuthorizeQuery) -> String {
    let escaped_state = html_escape(q.state.as_deref().unwrap_or(""));
    let escaped_client = html_escape(q.client_id.as_deref().unwrap_or(""));
    let escaped_redirect = html_escape(q.redirect_uri.as_deref().unwrap_or(""));
    let escaped_scope = html_escape(q.scope.as_deref().unwrap_or("openid"));
    let escaped_cc = html_escape(q.code_challenge.as_deref().unwrap_or(""));
    let escaped_ccm = html_escape(q.code_challenge_method.as_deref().unwrap_or("S256"));
    // Hidden fields carrying the downstream authorization request — repeated
    // in every form on the page so whichever the user submits round-trips it.
    let hidden = format!(
        r#"<input type="hidden" name="response_type" value="code">
<input type="hidden" name="client_id" value="{escaped_client}">
<input type="hidden" name="redirect_uri" value="{escaped_redirect}">
<input type="hidden" name="scope" value="{escaped_scope}">
<input type="hidden" name="state" value="{escaped_state}">
<input type="hidden" name="code_challenge" value="{escaped_cc}">
<input type="hidden" name="code_challenge_method" value="{escaped_ccm}">"#
    );

    // One form per configured upstream provider.
    let mut provider_forms = String::new();
    for p in state.providers.iter() {
        let key = html_escape(&p.key);
        let name = html_escape(&p.display_name);
        provider_forms.push_str(&format!(
            r#"<form method="post" action="/oauth/authorize">
{hidden}
<input type="hidden" name="provider" value="{key}">
<button type="submit">Sign in with {name}</button>
</form>"#
        ));
    }

    // The self-session paste form is always available (on-device fast path).
    let paste_form = format!(
        r#"<details{open}><summary>Advanced: paste a self-session token</summary>
<form method="post" action="/oauth/authorize">
{hidden}
<label>Self-session token <small>(from <code>ohd-storage-server issue-self-token</code>)</small><br>
<textarea name="self_session_token" rows="3"></textarea></label>
<button type="submit">Authorize</button>
</form></details>"#,
        // When no providers are configured the paste box is the only option,
        // so default it open.
        open = if state.providers.is_empty() { " open" } else { "" }
    );

    let intro = if state.providers.is_empty() {
        format!("<p>Paste your self-session token (<code>ohds_…</code>) to authorize <b>{escaped_client}</b>.</p>")
    } else {
        format!("<p>Choose how to sign in to authorize <b>{escaped_client}</b>.</p>")
    };

    format!(
        r#"<!doctype html><meta charset="utf-8"><title>OHD Storage — Sign in</title>
<style>body{{font-family:system-ui;max-width:48ch;margin:4em auto;padding:0 1em}}
input,textarea{{width:100%;font:inherit;padding:.5em;margin:.25em 0;box-sizing:border-box}}
button{{font:inherit;padding:.6em 1em;width:100%;margin:.25em 0}}
form{{margin:0 0 .5em}}
small{{color:#555}}</style>
<h1>Sign in to OHD Storage</h1>
{intro}
{provider_forms}
{paste_form}"#
    )
}

// ============================================================================
// /oauth/token
// ============================================================================

#[derive(Deserialize)]
struct TokenRequest {
    grant_type: Option<String>,
    code: Option<String>,
    redirect_uri: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    code_verifier: Option<String>,
    refresh_token: Option<String>,
    device_code: Option<String>,
    #[allow(dead_code)]
    scope: Option<String>,
}

#[derive(Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
}

async fn token_handler(State(state): State<OauthState>, Form(req): Form<TokenRequest>) -> Response {
    match req.grant_type.as_deref().unwrap_or("") {
        "authorization_code" => token_grant_authorization_code(&state, req),
        "refresh_token" => token_grant_refresh(&state, req),
        "urn:ietf:params:oauth:grant-type:device_code" => token_grant_device_code(&state, req),
        other => oauth_error_response(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            &format!("unsupported grant_type: {other:?}"),
        ),
    }
}

fn token_grant_authorization_code(state: &OauthState, req: TokenRequest) -> Response {
    let code = match req.code.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return oauth_error_response(StatusCode::BAD_REQUEST, "invalid_request", "missing code")
        }
    };
    let client_id = match req.client_id.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "missing client_id",
            )
        }
    };
    let redirect_uri = req.redirect_uri.unwrap_or_default();
    let code_verifier = match req.code_verifier.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "missing code_verifier (PKCE required)",
            )
        }
    };
    let code_hash = sha256_hex(&code);
    type Row = (
        i64,
        String,
        Vec<u8>,
        String,
        String,
        String,
        Option<String>,
        i64,
        Option<i64>,
    );
    let row: Result<Option<Row>, _> = state.storage.with_conn(|conn| {
        conn.query_row(
            "SELECT id, client_id, user_ulid, redirect_uri, scope, code_challenge,
                    nonce, expires_at_ms, used_at_ms
               FROM oauth_authorization_codes WHERE code_hash = ?1",
            params![code_hash],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                    r.get(7)?,
                    r.get(8)?,
                ))
            },
        )
        .optional()
        .map_err(ohd_storage_core::Error::from)
    });
    let row = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            return oauth_error_response(StatusCode::BAD_REQUEST, "invalid_grant", "code not found")
        }
        Err(e) => {
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &e.to_string(),
            )
        }
    };
    let (id, row_client, user_blob, row_redirect, scope, challenge, code_nonce, expires_at, used_at) =
        row;
    if used_at.is_some() {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "code already used",
        );
    }
    let now = ohd_storage_core::format::now_ms();
    if expires_at <= now {
        return oauth_error_response(StatusCode::BAD_REQUEST, "invalid_grant", "code expired");
    }
    if row_client != client_id {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "client_id mismatch",
        );
    }
    if row_redirect != redirect_uri {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "redirect_uri mismatch",
        );
    }
    // PKCE: SHA256(code_verifier) (b64url no-pad) MUST equal code_challenge.
    let computed = pkce_s256(&code_verifier);
    if computed != challenge {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "PKCE verification failed",
        );
    }
    // Mark used.
    let mark = state.storage.with_conn(|conn| {
        conn.execute(
            "UPDATE oauth_authorization_codes SET used_at_ms = ?1 WHERE id = ?2",
            params![now, id],
        )
        .map_err(ohd_storage_core::Error::from)
    });
    if let Err(e) = mark {
        return oauth_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        );
    }
    // Verify client (optionally — public clients may pass no secret).
    if let Err(resp) = require_client_secret(state, &client_id, req.client_secret.as_deref()) {
        return resp;
    }
    let user_ulid: [u8; 16] = match user_blob.as_slice().try_into() {
        Ok(u) => u,
        Err(_) => {
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "stored user_ulid not 16 bytes",
            )
        }
    };
    issue_token_pair(state, &client_id, user_ulid, &scope, true, code_nonce.as_deref())
}

fn token_grant_refresh(state: &OauthState, req: TokenRequest) -> Response {
    let refresh_token = match req.refresh_token.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "missing refresh_token",
            )
        }
    };
    let client_id = match req.client_id.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "missing client_id",
            )
        }
    };
    if let Err(resp) = require_client_secret(state, &client_id, req.client_secret.as_deref()) {
        return resp;
    }
    let hash = sha256_hex(&refresh_token);
    type Row = (i64, String, Vec<u8>, String, i64, Option<i64>);
    let row: Result<Option<Row>, _> = state.storage.with_conn(|conn| {
        conn.query_row(
            "SELECT id, client_id, user_ulid, scope, expires_at_ms, revoked_at_ms
               FROM oauth_refresh_tokens WHERE refresh_token_hash = ?1",
            params![hash],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .optional()
        .map_err(ohd_storage_core::Error::from)
    });
    let row = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "refresh_token not found",
            )
        }
        Err(e) => {
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &e.to_string(),
            )
        }
    };
    let (_id, row_client, user_blob, scope, expires_at, revoked_at) = row;
    if revoked_at.is_some() {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "refresh_token revoked",
        );
    }
    let now = ohd_storage_core::format::now_ms();
    if expires_at <= now {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "refresh_token expired",
        );
    }
    if row_client != client_id {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "client_id mismatch",
        );
    }
    let user_ulid: [u8; 16] = match user_blob.as_slice().try_into() {
        Ok(u) => u,
        Err(_) => {
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "stored user_ulid not 16 bytes",
            )
        }
    };
    // We re-issue an access token (and a fresh id_token); the original
    // refresh_token stays alive (rotation is a v1.x deliverable).
    issue_token_pair(state, &client_id, user_ulid, &scope, false, None)
}

fn token_grant_device_code(state: &OauthState, req: TokenRequest) -> Response {
    let device_code = match req.device_code.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "missing device_code",
            )
        }
    };
    let client_id = match req.client_id.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "missing client_id",
            )
        }
    };
    let hash = sha256_hex(&device_code);
    type Row = (
        i64,
        String,
        String,
        i64,
        Option<i64>,
        Option<Vec<u8>>,
        Option<i64>,
    );
    let row: Result<Option<Row>, _> = state.storage.with_conn(|conn| {
        conn.query_row(
            "SELECT id, client_id, scope, expires_at_ms, completed_at_ms,
                    completing_user_ulid, redeemed_at_ms
               FROM oauth_device_codes WHERE device_code_hash = ?1",
            params![hash],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()
        .map_err(ohd_storage_core::Error::from)
    });
    let row = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "device_code not found",
            )
        }
        Err(e) => {
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &e.to_string(),
            )
        }
    };
    let (id, row_client, scope, expires_at, completed_at, completing_user, redeemed_at) = row;
    let now = ohd_storage_core::format::now_ms();
    if expires_at <= now {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "expired_token",
            "device_code expired",
        );
    }
    if row_client != client_id {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "client_id mismatch",
        );
    }
    if redeemed_at.is_some() {
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "device_code already redeemed",
        );
    }
    if completed_at.is_none() || completing_user.is_none() {
        // Track polling rate; the client should respect `interval` returned by
        // /oauth/device. v0 doesn't enforce slow_down — we just report
        // authorization_pending.
        let _ = state.storage.with_conn(|conn| {
            conn.execute(
                "UPDATE oauth_device_codes SET last_polled_at_ms = ?1 WHERE id = ?2",
                params![now, id],
            )
            .map_err(ohd_storage_core::Error::from)
        });
        return oauth_error_response(
            StatusCode::BAD_REQUEST,
            "authorization_pending",
            "user has not completed device confirmation",
        );
    }
    let Some(user_blob) = completing_user else {
        return oauth_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "completing_user missing despite completed_at being set",
        );
    };
    let user_ulid: [u8; 16] = match user_blob.as_slice().try_into() {
        Ok(u) => u,
        Err(_) => {
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "stored user_ulid not 16 bytes",
            )
        }
    };
    let mark = state.storage.with_conn(|conn| {
        conn.execute(
            "UPDATE oauth_device_codes SET redeemed_at_ms = ?1 WHERE id = ?2",
            params![now, id],
        )
        .map_err(ohd_storage_core::Error::from)
    });
    if let Err(e) = mark {
        return oauth_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        );
    }
    issue_token_pair(state, &client_id, user_ulid, &scope, true, None)
}

fn issue_token_pair(
    state: &OauthState,
    client_id: &str,
    user_ulid: [u8; 16],
    scope: &str,
    issue_refresh: bool,
    nonce: Option<&str>,
) -> Response {
    let now = ohd_storage_core::format::now_ms();
    let access_token = format!("ohds_{}", mint_random_token());
    let access_hash = ohd_auth::hash_token(&access_token).to_vec();
    let access_ttl_ms = ACCESS_TOKEN_TTL_S * 1000;
    let refresh_ttl_ms = REFRESH_TOKEN_TTL_S * 1000;
    // Persist the access token in the storage's `_tokens` table so it can be
    // used as a regular OHDC self-session bearer immediately. Label it for
    // audit visibility.
    let access_label = format!("oauth:{client_id}");
    let access_persist = state.storage.with_conn(|conn| {
        conn.execute(
            "INSERT INTO _tokens (token_prefix, token_hash, user_ulid, grant_id,
                                  issued_at_ms, expires_at_ms, label)
             VALUES ('ohds', ?1, ?2, NULL, ?3, ?4, ?5)",
            params![
                access_hash,
                user_ulid.to_vec(),
                now,
                now + access_ttl_ms,
                access_label
            ],
        )
        .map_err(ohd_storage_core::Error::from)
    });
    if let Err(e) = access_persist {
        return oauth_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        );
    }
    let refresh_token = if issue_refresh {
        let rt = format!("ohdr_{}", mint_random_token());
        let rt_hash = sha256_hex(&rt);
        let r = state.storage.with_conn(|conn| {
            conn.execute(
                "INSERT INTO oauth_refresh_tokens
                    (refresh_token_hash, client_id, user_ulid, scope,
                     issued_at_ms, expires_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    rt_hash,
                    client_id,
                    user_ulid.to_vec(),
                    scope,
                    now,
                    now + refresh_ttl_ms
                ],
            )
            .map_err(ohd_storage_core::Error::from)
        });
        if let Err(e) = r {
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &e.to_string(),
            );
        }
        Some(rt)
    } else {
        None
    };
    let id_token = match signing::mint_id_token(
        &state.storage,
        &state.issuer,
        client_id,
        user_ulid,
        now,
        access_ttl_ms,
        nonce,
    ) {
        Ok(t) => Some(t),
        Err(e) => {
            return oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &format!("id_token mint: {e}"),
            )
        }
    };
    let body = TokenResponse {
        access_token,
        token_type: "Bearer",
        expires_in: ACCESS_TOKEN_TTL_S,
        refresh_token,
        id_token,
        scope: if scope.is_empty() {
            None
        } else {
            Some(scope.to_string())
        },
    };
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    headers.insert(header::PRAGMA, "no-cache".parse().unwrap());
    (StatusCode::OK, headers, Json(body)).into_response()
}

// ============================================================================
// /oauth/device  — Device Authorization Grant (RFC 8628 §3.1)
// ============================================================================

#[derive(Deserialize)]
struct DeviceAuthorizeRequest {
    client_id: Option<String>,
    scope: Option<String>,
}

#[derive(Serialize)]
struct DeviceAuthorizeResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    expires_in: i64,
    interval: i64,
}

async fn device_authorize_handler(
    State(state): State<OauthState>,
    Form(req): Form<DeviceAuthorizeRequest>,
) -> Response {
    let client_id = match req.client_id.as_deref() {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "missing client_id",
            )
        }
    };
    let scope = req.scope.unwrap_or_else(|| "openid".into());
    let device_code = mint_random_token();
    let user_code = mint_user_code();
    let device_hash = sha256_hex(&device_code);
    let now = ohd_storage_core::format::now_ms();
    let r = state.storage.with_conn(|conn| {
        conn.execute(
            "INSERT INTO oauth_device_codes
                (device_code_hash, user_code, client_id, scope,
                 issued_at_ms, expires_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                device_hash,
                user_code,
                client_id,
                scope,
                now,
                now + DEVICE_CODE_TTL_S * 1000,
            ],
        )
        .map_err(ohd_storage_core::Error::from)
    });
    if let Err(e) = r {
        return oauth_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        );
    }
    let issuer = state.issuer.trim_end_matches('/');
    let verification_uri = format!("{issuer}/oauth/device-confirm");
    let verification_uri_complete = format!("{verification_uri}?user_code={user_code}");
    let body = DeviceAuthorizeResponse {
        device_code,
        user_code,
        verification_uri,
        verification_uri_complete,
        expires_in: DEVICE_CODE_TTL_S,
        interval: DEVICE_CODE_POLL_INTERVAL_S,
    };
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    (StatusCode::OK, headers, Json(body)).into_response()
}

// ============================================================================
// /oauth/device-confirm  — User-facing confirmation page
// ============================================================================

#[derive(Deserialize)]
struct DeviceConfirmQuery {
    user_code: Option<String>,
}

#[derive(Deserialize)]
struct DeviceConfirmPost {
    user_code: Option<String>,
    self_session_token: Option<String>,
}

async fn device_confirm_get(Query(q): Query<DeviceConfirmQuery>) -> Response {
    let escaped = html_escape(q.user_code.as_deref().unwrap_or(""));
    let html = format!(
        r#"<!doctype html><meta charset="utf-8"><title>OHD Storage — Device login</title>
<style>body{{font-family:system-ui;max-width:48ch;margin:4em auto;padding:0 1em}}
input,textarea{{width:100%;font:inherit;padding:.5em;margin:.25em 0;box-sizing:border-box}}
button{{font:inherit;padding:.6em 1em}}
small{{color:#555}}</style>
<h1>Confirm device login</h1>
<p>Enter the code shown by the CLI / device, plus your self-session token to authorize it.</p>
<form method="post" action="/oauth/device-confirm">
<label>User code <input name="user_code" value="{escaped}" required pattern="[A-Z0-9-]+" autocomplete="off"></label>
<label>Self-session token <small>(<code>ohds_…</code>)</small>
<textarea name="self_session_token" rows="3" required></textarea></label>
<button type="submit">Approve</button>
</form>"#
    );
    (StatusCode::OK, Html(html)).into_response()
}

async fn device_confirm_post(
    State(state): State<OauthState>,
    Form(f): Form<DeviceConfirmPost>,
) -> Response {
    let user_code = match f.user_code.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => s.to_uppercase(),
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "missing user_code",
            )
        }
    };
    let bearer = match f.self_session_token.as_deref().map(str::trim) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "access_denied",
                "missing self-session token",
            )
        }
    };
    let resolved = match state
        .storage
        .with_conn(|conn| ohd_auth::resolve_token(conn, &bearer))
    {
        Ok(r) if r.kind == ohd_auth::TokenKind::SelfSession => r,
        Ok(_) => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "access_denied",
                "only self-session tokens accepted",
            )
        }
        Err(e) => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "access_denied",
                &format!("invalid self-session token: {e}"),
            )
        }
    };
    let now = ohd_storage_core::format::now_ms();
    let r = state.storage.with_conn(|conn| {
        let updated = conn.execute(
            "UPDATE oauth_device_codes
                SET completed_at_ms = ?1,
                    completing_user_ulid = ?2
              WHERE user_code = ?3
                AND completed_at_ms IS NULL
                AND expires_at_ms > ?1",
            params![now, resolved.user_ulid.to_vec(), user_code],
        )?;
        Ok::<usize, ohd_storage_core::Error>(updated)
    });
    match r {
        Ok(0) => oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "user_code unknown or expired",
        ),
        Ok(_) => (
            StatusCode::OK,
            Html("<!doctype html><meta charset=\"utf-8\"><title>OK</title><p>Device approved. You can close this tab.</p>"),
        )
            .into_response(),
        Err(e) => oauth_error_response(StatusCode::INTERNAL_SERVER_ERROR, "server_error", &e.to_string()),
    }
}

// ============================================================================
// /oauth/userinfo
// ============================================================================

async fn userinfo_handler(State(state): State<OauthState>, headers: HeaderMap) -> Response {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    let bearer = match bearer {
        Some(b) if !b.is_empty() => b.to_string(),
        _ => {
            return oauth_error_response(
                StatusCode::UNAUTHORIZED,
                "invalid_token",
                "missing bearer access_token",
            )
        }
    };
    let resolved = match state
        .storage
        .with_conn(|conn| ohd_auth::resolve_token(conn, &bearer))
    {
        Ok(r) => r,
        Err(e) => {
            return oauth_error_response(
                StatusCode::UNAUTHORIZED,
                "invalid_token",
                &format!("token: {e}"),
            )
        }
    };
    let identities = state
        .storage
        .with_conn(|conn| ohd_storage_core::identities::list_identities(conn, resolved.user_ulid))
        .unwrap_or_default();
    #[derive(Serialize)]
    struct LinkedIdentity {
        provider: String,
        subject: String,
        primary: bool,
    }
    #[derive(Serialize)]
    struct UserInfo {
        sub: String,
        linked_identities: Vec<LinkedIdentity>,
        primary_provider: Option<String>,
        primary_subject: Option<String>,
    }
    let primary = identities.iter().find(|i| i.is_primary);
    let body = UserInfo {
        sub: ohd_storage_core::ulid::to_crockford(&resolved.user_ulid),
        primary_provider: primary.map(|p| p.provider.clone()),
        primary_subject: primary.map(|p| p.subject.clone()),
        linked_identities: identities
            .iter()
            .map(|i| LinkedIdentity {
                provider: i.provider.clone(),
                subject: i.subject.clone(),
                primary: i.is_primary,
            })
            .collect(),
    };
    (StatusCode::OK, Json(body)).into_response()
}

// ============================================================================
// /oauth/register  — Dynamic Client Registration (RFC 7591, minimal)
// ============================================================================

#[derive(Deserialize)]
struct RegisterRequest {
    client_name: Option<String>,
    redirect_uris: Option<Vec<String>>,
    grant_types: Option<Vec<String>>,
    response_types: Option<Vec<String>>,
    /// `none` for public clients (PKCE); `client_secret_post` for confidentials.
    token_endpoint_auth_method: Option<String>,
}

#[derive(Serialize)]
struct RegisterResponse {
    client_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_secret: Option<String>,
    client_id_issued_at: i64,
    client_name: String,
    redirect_uris: Vec<String>,
    grant_types: Vec<String>,
    response_types: Vec<String>,
    token_endpoint_auth_method: String,
}

async fn register_handler(
    State(state): State<OauthState>,
    headers: HeaderMap,
    Json(req): Json<RegisterRequest>,
) -> Response {
    // v0: registration requires a self-session token from the operator. This
    // makes the path safe to expose on a public host without becoming an open
    // sign-up — the operator (the storage owner) is the only entity that can
    // create clients. A future v1.x can relax this for `open` deployment mode.
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    let bearer = match bearer {
        Some(b) if !b.is_empty() => b.to_string(),
        _ => {
            return oauth_error_response(
                StatusCode::UNAUTHORIZED,
                "invalid_token",
                "client registration requires a self-session bearer token",
            )
        }
    };
    let resolved = match state
        .storage
        .with_conn(|conn| ohd_auth::resolve_token(conn, &bearer))
    {
        Ok(r) if r.kind == ohd_auth::TokenKind::SelfSession => r,
        _ => {
            return oauth_error_response(
                StatusCode::UNAUTHORIZED,
                "invalid_token",
                "only self-session tokens may register clients",
            )
        }
    };
    let client_name = req.client_name.unwrap_or_else(|| "ohd-client".into());
    let redirect_uris = req.redirect_uris.unwrap_or_default();
    let grant_types = req
        .grant_types
        .unwrap_or_else(|| vec!["authorization_code".into(), "refresh_token".into()]);
    let response_types = req.response_types.unwrap_or_else(|| vec!["code".into()]);
    let auth_method = req
        .token_endpoint_auth_method
        .unwrap_or_else(|| "none".into());
    let client_id = format!("ohdc_{}", mint_random_token());
    let (client_secret, secret_hash): (Option<String>, Option<Vec<u8>>) = match auth_method.as_str()
    {
        "none" => (None, None),
        "client_secret_post" => {
            let s = mint_random_token();
            let h = sha256_hex(&s);
            (Some(s), Some(h))
        }
        other => {
            return oauth_error_response(
                StatusCode::BAD_REQUEST,
                "invalid_client_metadata",
                &format!("unsupported token_endpoint_auth_method: {other:?}"),
            )
        }
    };
    let now = ohd_storage_core::format::now_ms();
    let redirect_json = serde_json::to_string(&redirect_uris).unwrap_or_default();
    let grant_csv = grant_types.join(",");
    let resp_csv = response_types.join(",");
    let r = state.storage.with_conn(|conn| {
        conn.execute(
            "INSERT INTO oauth_clients
                (client_id, client_name, client_secret_hash, redirect_uris,
                 grant_types_csv, response_types_csv, created_at_ms, created_by_user_ulid)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                client_id,
                client_name,
                secret_hash,
                redirect_json,
                grant_csv,
                resp_csv,
                now,
                resolved.user_ulid.to_vec(),
            ],
        )
        .map_err(ohd_storage_core::Error::from)
    });
    if let Err(e) = r {
        return oauth_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        );
    }
    let body = RegisterResponse {
        client_id,
        client_secret,
        client_id_issued_at: now / 1000,
        client_name,
        redirect_uris,
        grant_types,
        response_types,
        token_endpoint_auth_method: auth_method,
    };
    (StatusCode::CREATED, Json(body)).into_response()
}

// ============================================================================
// Helpers
// ============================================================================

fn require_client_redirect(
    state: &OauthState,
    client_id: &str,
    redirect_uri: &str,
) -> Result<(), Response> {
    let row: Result<Option<String>, _> = state.storage.with_conn(|conn| {
        conn.query_row(
            "SELECT redirect_uris FROM oauth_clients WHERE client_id = ?1",
            params![client_id],
            |r| r.get::<_, String>(0),
        )
        .optional()
        .map_err(ohd_storage_core::Error::from)
    });
    match row {
        Ok(Some(json)) => {
            let uris: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
            if uris.iter().any(|u| u == redirect_uri) {
                Ok(())
            } else {
                Err(oauth_error_response(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "redirect_uri not registered for client",
                ))
            }
        }
        Ok(None) => Err(oauth_error_response(
            StatusCode::BAD_REQUEST,
            "invalid_client",
            "unknown client_id",
        )),
        Err(e) => Err(oauth_error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            &e.to_string(),
        )),
    }
}

fn require_client_secret(
    state: &OauthState,
    client_id: &str,
    presented_secret: Option<&str>,
) -> Result<(), Response> {
    let row: Result<Option<Option<Vec<u8>>>, _> = state.storage.with_conn(|conn| {
        conn.query_row(
            "SELECT client_secret_hash FROM oauth_clients WHERE client_id = ?1",
            params![client_id],
            |r| r.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()
        .map_err(ohd_storage_core::Error::from)
    });
    let stored = match row {
        Ok(Some(s)) => s,
        Ok(None) => {
            return Err(oauth_error_response(
                StatusCode::UNAUTHORIZED,
                "invalid_client",
                "unknown client_id",
            ))
        }
        Err(e) => {
            return Err(oauth_error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                &e.to_string(),
            ))
        }
    };
    match (stored, presented_secret) {
        (None, _) => Ok(()), // Public client: no secret required.
        (Some(_expected), None) => Err(oauth_error_response(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "client_secret required",
        )),
        (Some(expected), Some(presented)) => {
            let computed = sha256_hex(presented);
            if computed == expected {
                Ok(())
            } else {
                Err(oauth_error_response(
                    StatusCode::UNAUTHORIZED,
                    "invalid_client",
                    "client_secret mismatch",
                ))
            }
        }
    }
}

#[derive(Serialize)]
struct OauthErrorBody<'a> {
    error: &'a str,
    error_description: &'a str,
}

fn oauth_error_response(status: StatusCode, error: &str, description: &str) -> Response {
    let body = OauthErrorBody {
        error,
        error_description: description,
    };
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    headers.insert(header::PRAGMA, "no-cache".parse().unwrap());
    (status, headers, Json(body)).into_response()
}

/// 32 bytes of CSPRNG, b64url no-pad encoded.
fn mint_random_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64_url_no_pad(&bytes)
}

/// User-facing device code: 8 chars, mixed letters/digits, hyphenated for
/// readability (e.g. `ABCD-WXYZ`). Avoids ambiguous chars (0/O, 1/I/L).
fn mint_user_code() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";
    let mut rng = rand::thread_rng();
    let mut s = String::with_capacity(9);
    for i in 0..8 {
        if i == 4 {
            s.push('-');
        }
        let idx = rng.gen_range(0..ALPHABET.len());
        s.push(ALPHABET[idx] as char);
    }
    s
}

fn sha256_hex(s: &str) -> Vec<u8> {
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().to_vec()
}

fn pkce_s256(code_verifier: &str) -> String {
    let mut h = Sha256::new();
    h.update(code_verifier.as_bytes());
    let digest = h.finalize();
    base64_url_no_pad(&digest)
}

fn base64_url_no_pad(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}
