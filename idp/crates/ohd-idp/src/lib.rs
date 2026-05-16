//! OHD Identity — the OHD project's OpenID Connect provider.
//!
//! `ohd-idp` is the OIDC OP deployed at `accounts.ohd.dev` that OHD CORD,
//! OHD Connect, and future OHD apps authenticate users against. See
//! [`SPEC.md`](../../SPEC.md).
//!
//! This crate is **Phase 1** — the service skeleton: config loading,
//! RS256 signing-key management, the JWKS + OIDC discovery endpoints, the
//! RP-client registry, and `/healthz`. The `/authorize`, `/login`,
//! `/token`, and `/userinfo` flows are later phases.
//!
//! Production callers depend on [`server::build_router`] + [`config`] +
//! [`keys::SigningKey`]; the rest is re-exported for tests.

pub mod config;
pub mod discovery;
pub mod errors;
pub mod jwks;
pub mod keys;
pub mod registry;
pub mod routes;
pub mod server;

pub use config::Config;
pub use keys::SigningKey;
pub use registry::ClientRegistry;
pub use server::build_router;
