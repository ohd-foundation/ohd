//! Lightweight hand-written HTML docs served at `/docs`.
//!
//! Mirrors [`SPEC.md`](../SPEC.md). Kept inline (no external static asset)
//! so the service ships as a single binary and the docs travel with the
//! versioned source — every release describes its own surface.

use axum::response::Html;

const DOCS_HTML: &str = include_str!("docs.html");

pub async fn docs() -> Html<&'static str> {
    Html(DOCS_HTML)
}
