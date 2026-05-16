//! Metadata + liveness endpoints: OIDC discovery, JWKS, and `/healthz`.

use crate::discovery::Discovery;
use crate::server::AppState;
use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

/// `GET /healthz` — liveness. Returns 200 with a small status body.
pub async fn healthz() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "service": "ohd-idp",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

/// `GET /.well-known/openid-configuration` — the OIDC discovery document.
/// Endpoint URLs are derived from the configured issuer.
pub async fn discovery(State(app): State<AppState>) -> Json<Discovery> {
    Json(Discovery::for_issuer(&app.config.server.issuer))
}

/// `GET /jwks` — the JSON Web Key Set: the active RS256 public key plus
/// every non-expired rotation-overlap key, so an `id_token` signed under a
/// recently-rotated key still verifies.
pub async fn jwks(State(app): State<AppState>) -> Json<crate::jwks::Jwks> {
    Json(app.keys.jwks())
}
