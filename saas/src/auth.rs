//! Bearer-token plumbing. HS256 JWTs scoped to a single `profile_ulid`.

use crate::errors::ApiError;
use axum::extract::{FromRef, FromRequestParts};
use axum::http::request::Parts;
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub iat: i64,
    pub exp: i64,
    pub iss: String,
}

const ISSUER: &str = "ohd-saas";

pub fn mint_token(profile_ulid: &str, secret: &str, ttl_days: i64) -> Result<String, ApiError> {
    let now = now_s();
    let exp = now + ttl_days * 86_400;
    let claims = Claims {
        sub: profile_ulid.to_string(),
        iat: now,
        exp,
        iss: ISSUER.to_string(),
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| ApiError::Internal(anyhow::anyhow!(e)))
}

pub fn verify_token(token: &str, secret: &str) -> Result<Claims, ApiError> {
    let mut validation = Validation::default();
    validation.set_issuer(&[ISSUER]);
    let data = decode::<Claims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map_err(|_| ApiError::Unauthorized)?;
    Ok(data.claims)
}

pub struct AuthedProfile(pub String);

#[axum::async_trait]
impl<S> FromRequestParts<S> for AuthedProfile
where
    S: Send + Sync,
    crate::server::AppState: axum::extract::FromRef<S>,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let bearer = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.strip_prefix("Bearer "))
            .ok_or(ApiError::Unauthorized)?;
        let app_state = crate::server::AppState::from_ref(state);
        let claims = verify_token(bearer, &app_state.config.jwt_secret)?;
        Ok(AuthedProfile(claims.sub))
    }
}

fn now_s() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
