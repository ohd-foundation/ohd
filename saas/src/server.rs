use crate::config::Config;
use crate::db::Db;
use crate::docs;
use crate::routes;
use axum::routing::{delete, get, post};
use axum::Router;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    pub config: Config,
}

pub fn build_router(db: Db, config: Config) -> Router {
    let state = AppState { db, config };
    Router::new()
        .route("/healthz", get(routes::healthz))
        .route("/docs", get(docs::docs))
        .route("/", get(docs::docs))
        .route("/v1/account", post(routes::register))
        .route("/v1/account/me", get(routes::me))
        .route("/v1/account/recover", post(routes::recover))
        .route("/v1/account/oidc/link", post(routes::link_oidc))
        .route("/v1/account/oidc", delete(routes::unlink_oidc))
        .route("/v1/account/oidc/claim", post(routes::claim_oidc))
        .route("/v1/account/plan", get(routes::current_plan))
        .route("/v1/account/plan/checkout", post(routes::checkout))
        .route("/v1/account/payments", get(routes::list_payments))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any))
}
