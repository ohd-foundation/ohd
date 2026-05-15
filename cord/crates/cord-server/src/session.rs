//! CORD sessions — stateless HS256 JWTs scoped to one `cord_user_ulid`.
//! Browsers carry the session in an httpOnly `cord_session` cookie; API
//! clients may use `Authorization: Bearer` instead.

use crate::errors::ApiError;
use crate::server::AppState;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::header::{AUTHORIZATION, COOKIE};
use axum::http::request::Parts;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

const ISSUER: &str = "ohd-cord";
pub const COOKIE_NAME: &str = "cord_session";

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iat: i64,
    pub exp: i64,
    pub iss: String,
}

pub fn mint(user_ulid: &str, secret: &str, ttl_hours: i64) -> Result<String, ApiError> {
    let now = now_s();
    let claims = Claims {
        sub: user_ulid.to_string(),
        iat: now,
        exp: now + ttl_hours * 3600,
        iss: ISSUER.to_string(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))
}

pub fn verify(token: &str, secret: &str) -> Result<Claims, ApiError> {
    let mut validation = Validation::default();
    validation.set_issuer(&[ISSUER]);
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|d| d.claims)
    .map_err(|_| ApiError::Unauthorized)
}

/// Build the `Set-Cookie` value for a freshly minted session.
pub fn cookie_for(token: &str, ttl_hours: i64, secure: bool) -> String {
    let max_age = ttl_hours * 3600;
    let secure_attr = if secure { "; Secure" } else { "" };
    format!(
        "{COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure_attr}"
    )
}

/// `Set-Cookie` value that clears the session.
pub fn clear_cookie(secure: bool) -> String {
    let secure_attr = if secure { "; Secure" } else { "" };
    format!("{COOKIE_NAME}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure_attr}")
}

/// Extractor: the authenticated `cord_user_ulid`. Rejects with 401 when no
/// valid session is present.
pub struct CurrentUser(pub String);

#[axum::async_trait]
impl<S> FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
    AppState: FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let app = AppState::from_ref(state);
        let token = bearer(parts)
            .or_else(|| cookie_token(parts))
            .ok_or(ApiError::Unauthorized)?;
        let claims = verify(&token, &app.config.session_secret)?;
        Ok(CurrentUser(claims.sub))
    }
}

fn bearer(parts: &Parts) -> Option<String> {
    parts
        .headers
        .get(AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.to_string())
}

fn cookie_token(parts: &Parts) -> Option<String> {
    let raw = parts.headers.get(COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        let pair = pair.trim();
        if let Some(v) = pair.strip_prefix(&format!("{COOKIE_NAME}=")) {
            return Some(v.to_string());
        }
    }
    None
}

fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
