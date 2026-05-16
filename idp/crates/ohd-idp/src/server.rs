//! Router assembly + shared application state.

use crate::codes::IdpStore;
use crate::config::Config;
use crate::keys::SigningKey;
use crate::registry::ClientRegistry;
use crate::routes;
use crate::store::AccountStore;
use axum::routing::{get, post};
use axum::Router;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

/// Cloneable per-request state. `Config` is `Arc`-wrapped so cloning is
/// cheap; the keys, registry, and the two stores are already cheap to
/// clone (`Arc`-backed).
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub signing_key: SigningKey,
    pub clients: ClientRegistry,
    /// The shared OHD SaaS account store — email/password credentials.
    pub accounts: AccountStore,
    /// The IdP-local store — authorization codes + access tokens.
    pub idp_store: IdpStore,
}

/// Assemble the axum router from a resolved config, a loaded signing key,
/// and the two opened stores.
pub fn build_router(
    config: Config,
    signing_key: SigningKey,
    accounts: AccountStore,
    idp_store: IdpStore,
) -> Router {
    let clients = ClientRegistry::from_config(&config.clients);
    let state = AppState {
        config: Arc::new(config),
        signing_key,
        clients,
        accounts,
        idp_store,
    };

    Router::new()
        .route("/healthz", get(routes::meta::healthz))
        .route(
            "/.well-known/openid-configuration",
            get(routes::meta::discovery),
        )
        .route("/jwks", get(routes::meta::jwks))
        .route("/authorize", get(routes::oidc::authorize))
        .route("/login", get(routes::oidc::login_form).post(routes::oidc::login_submit))
        .route(
            "/signup",
            get(routes::oidc::signup_form).post(routes::oidc::signup_submit),
        )
        .route("/continue", get(routes::oidc::continue_flow))
        .route("/token", post(routes::oidc::token))
        .route("/userinfo", get(routes::oidc::userinfo))
        .with_state(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}
