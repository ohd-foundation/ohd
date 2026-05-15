//! API error type. One `IntoResponse` impl maps every failure mode to a
//! clean JSON `{ "error": "..." }` body with the right status code.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden: {0}")]
    Forbidden(String),
    #[error("conflict")]
    Conflict,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not implemented: {0}")]
    NotImplemented(String),
    #[error("upstream error: {0}")]
    Upstream(String),
    #[error("db error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("pool error: {0}")]
    Pool(#[from] r2d2::Error),
    #[error("internal: {0}")]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            ApiError::NotFound => (StatusCode::NOT_FOUND, self.to_string()),
            ApiError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            ApiError::Forbidden(_) => (StatusCode::FORBIDDEN, self.to_string()),
            ApiError::Conflict => (StatusCode::CONFLICT, self.to_string()),
            ApiError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            ApiError::NotImplemented(_) => (StatusCode::NOT_IMPLEMENTED, self.to_string()),
            ApiError::Upstream(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            ApiError::Db(_) | ApiError::Pool(_) | ApiError::Internal(_) => {
                tracing::error!(error = %self, "internal error");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal error".to_string())
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
