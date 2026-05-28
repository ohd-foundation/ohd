//! Router assembly + shared application state.

use crate::config::Config;
use crate::db::Db;
use crate::oidc::{new_pending, OidcClient, PendingLogins};
use crate::routes;
use axum::routing::{delete, get, post};
use axum::Router;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

/// Cloneable per-request state. `Config` is `Arc`-wrapped so cloning is
/// cheap; `Db` and `OidcClient` are already cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Arc<Config>,
    pub oidc: OidcClient,
    pub pending: PendingLogins,
}

pub fn build_router(db: Db, config: Config) -> Router {
    let web_dir = config.web_dir.clone();
    let state = AppState {
        db,
        config: Arc::new(config),
        oidc: OidcClient::new(),
        pending: new_pending(),
    };

    let mut router = Router::new()
        .route("/healthz", get(routes::misc::healthz))
        .route("/v1/me", get(routes::misc::me))
        .route("/v1/auth/providers", get(routes::auth::providers))
        .route("/v1/auth/start", get(routes::auth::start))
        .route("/v1/auth/callback", get(routes::auth::callback))
        .route("/v1/auth/logout", post(routes::auth::logout))
        .route("/v1/sources", get(routes::sources::list))
        .route("/v1/sources/connect", post(routes::sources::connect))
        .route(
            "/v1/sources/:id",
            get(routes::sources::get_one)
                .patch(routes::sources::rename)
                .delete(routes::sources::delete_one),
        )
        .route("/v1/sources/:id/refresh", post(routes::sources::refresh))
        .route("/v1/sources/:id/summary", get(routes::sources::summary))
        .route("/v1/models", get(routes::models::list))
        .route("/v1/models/byo", post(routes::models::add_byo))
        .route("/v1/models/byo/:id", delete(routes::models::delete_byo))
        .route(
            "/v1/chats",
            get(routes::chats::list).post(routes::chats::create),
        )
        .route(
            "/v1/chats/:id",
            get(routes::chats::get_one).delete(routes::chats::delete_one),
        )
        .route("/v1/chats/:id/messages", post(routes::chats::send_message))
        .with_state(state);

    // When this deployment serves the bundled SPA, anything not matched
    // above falls through to the static assets — and any path that is not
    // a real file (a client-side route like `/connections/<id>`) falls
    // back to `index.html` so a hard refresh / deep link loads the SPA
    // instead of 404ing.
    if let Some(dir) = web_dir {
        let index = format!("{}/index.html", dir.trim_end_matches('/'));
        router = router.fallback_service(
            ServeDir::new(&dir).fallback(ServeFile::new(index)),
        );
    }

    router
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}
