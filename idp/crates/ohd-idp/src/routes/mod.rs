//! HTTP route handlers.
//!
//! Phase 1 implements only the metadata + liveness surface:
//! `/.well-known/openid-configuration`, `/jwks`, and `/healthz`. The
//! `/authorize`, `/login`, `/token`, and `/userinfo` flows are later
//! phases and are not routed yet.

pub mod meta;
