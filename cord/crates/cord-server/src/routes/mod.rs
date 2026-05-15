//! HTTP route handlers, grouped by feature.

pub mod auth;
pub mod chats;
pub mod misc;
pub mod models;
pub mod sources;

use axum::body::Body;
use axum::http::header::{LOCATION, SET_COOKIE};
use axum::http::StatusCode;
use axum::response::Response;

/// A bare 302 redirect.
pub(crate) fn redirect_to(location: &str) -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(LOCATION, location)
        .body(Body::empty())
        .expect("static redirect response is valid")
}

/// A 302 redirect that also sets (or clears) the session cookie.
pub(crate) fn redirect_with_cookie(location: &str, cookie: &str) -> Response {
    Response::builder()
        .status(StatusCode::FOUND)
        .header(LOCATION, location)
        .header(SET_COOKIE, cookie)
        .body(Body::empty())
        .expect("static redirect response is valid")
}
