//! Router assembly + shared application state.

use crate::config::Config;
use crate::keys::SigningKey;
use crate::registry::ClientRegistry;
use crate::routes;
use axum::routing::get;
use axum::Router;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

/// Cloneable per-request state. `Config` is `Arc`-wrapped so cloning is
/// cheap; `SigningKey` and `ClientRegistry` are already cheap to clone.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub signing_key: SigningKey,
    pub clients: ClientRegistry,
}

/// Assemble the axum router from a resolved config and a loaded signing
/// key. The RP-client registry is built from the config's `[[client]]`
/// entries.
pub fn build_router(config: Config, signing_key: SigningKey) -> Router {
    let clients = ClientRegistry::from_config(&config.clients);
    let state = AppState {
        config: Arc::new(config),
        signing_key,
        clients,
    };

    Router::new()
        .route("/healthz", get(routes::meta::healthz))
        .route(
            "/.well-known/openid-configuration",
            get(routes::meta::discovery),
        )
        .route("/jwks", get(routes::meta::jwks))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}
