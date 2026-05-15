//! OHD SaaS — account / plan / billing service.
//!
//! See [`SPEC.md`](../SPEC.md) for the surface. The `lib.rs` re-exports
//! the bits unit tests need; production callers should depend on
//! [`server::build_router`] only.

pub mod auth;
pub mod config;
pub mod db;
pub mod docs;
pub mod errors;
pub mod plans;
pub mod routes;
pub mod server;

pub use config::Config;
pub use db::Db;
pub use server::build_router;
