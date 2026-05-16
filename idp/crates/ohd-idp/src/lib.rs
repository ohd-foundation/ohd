//! OHD Identity — the OHD project's OpenID Connect provider.
//!
//! `ohd-idp` is the OIDC OP deployed at `accounts.ohd.dev` that OHD CORD,
//! OHD Connect, and future OHD apps authenticate users against. See
//! [`SPEC.md`](../../SPEC.md).
//!
//! Phases 1–2 are implemented here: config loading, RS256 signing-key
//! management, the JWKS + OIDC discovery endpoints, the RP-client
//! registry, `/healthz` (Phase 1), and the full email/password
//! authorization-code flow — `/authorize`, the SSR login + sign-up UI,
//! `/token`, `/userinfo` (Phase 2). Recovery-code login, the bounded SSO
//! session, and logout are Phase 3.
//!
//! Production callers depend on [`server::build_router`] + [`config`] +
//! [`keys::SigningKey`] + [`store::AccountStore`] + [`codes::IdpStore`];
//! the rest is re-exported for tests.

pub mod codes;
pub mod config;
pub mod discovery;
pub mod errors;
pub mod html;
pub mod jwks;
pub mod keys;
pub mod registry;
pub mod routes;
pub mod server;
pub mod store;
pub mod token;

pub use codes::IdpStore;
pub use config::Config;
pub use keys::SigningKey;
pub use registry::ClientRegistry;
pub use server::build_router;
pub use store::AccountStore;
