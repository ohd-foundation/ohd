//! HTTP route handlers.
//!
//! - [`meta`] — the metadata + liveness surface: OIDC discovery, `/jwks`,
//!   `/healthz`.
//! - [`oidc`] — Phase 2's OIDC authorization-code flow: `/authorize`, the
//!   SSR login + sign-up UI, `/token`, `/userinfo`.

pub mod meta;
pub mod oidc;
