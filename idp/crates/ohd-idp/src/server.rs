//! Router assembly + shared application state.

use crate::codes::IdpStore;
use crate::config::Config;
use crate::keystore::KeyStore;
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
    /// The IdP's signing keys — the active key plus rotation-overlap keys.
    pub keys: KeyStore,
    pub clients: ClientRegistry,
    /// The shared OHD SaaS account store — email/password credentials.
    pub accounts: AccountStore,
    /// The IdP-local store — authorization codes + access tokens + SSO
    /// sessions.
    pub idp_store: IdpStore,
}

/// Assemble the axum router from a resolved config, the loaded signing
/// keys, and the two opened stores.
pub fn build_router(
    config: Config,
    keys: KeyStore,
    accounts: AccountStore,
    idp_store: IdpStore,
) -> Router {
    let clients = ClientRegistry::from_config(&config.clients);
    let state = AppState {
        config: Arc::new(config),
        keys,
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
        .route(
            "/reset",
            get(routes::oidc::reset_form).post(routes::oidc::reset_submit),
        )
        .route("/continue", get(routes::oidc::continue_flow))
        .route("/token", post(routes::oidc::token))
        .route("/userinfo", get(routes::oidc::userinfo))
        .route(
            "/logout",
            get(routes::oidc::logout).post(routes::oidc::logout),
        )
        .with_state(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
}
