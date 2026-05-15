//! OHD CORD server — the conversational agent web service backend.
//!
//! See [`SPEC.md`](../../SPEC.md) and [`spec/data-link.md`](../../spec/data-link.md).
//! Production callers depend on [`server::build_router`] + [`config`] +
//! [`db::Db`]; the rest is re-exported for tests.

pub mod config;
pub mod crypto;
pub mod db;
pub mod errors;
pub mod oidc;
pub mod routes;
pub mod server;
pub mod session;
pub mod share;
pub mod util;

pub use config::Config;
pub use db::Db;
pub use server::build_router;
