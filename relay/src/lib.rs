//! OHD Relay library entry point.
//!
//! Exposes the internals (frame codec, state tables, server router) so they
//! can be exercised by integration tests in `tests/`. The binary at
//! `src/main.rs` consumes the same modules.

pub mod auth;
pub mod auth_mode;
pub mod config;
pub mod emergency_endpoints;
pub mod frame;
pub mod http3;
pub mod pairing;
pub mod push;
pub mod quic_tunnel;
pub mod server;
pub mod session;
pub mod state;
