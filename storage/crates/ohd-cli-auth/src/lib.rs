//! Shared CLI helpers for the OHD command-line tools.
//!
//! Two unrelated concerns share this crate by virtue of being needed by
//! both `ohd-connect` and `ohd-emergency`:
//!
//! - [`oidc`] — OAuth 2.0 Device Authorization Grant client (RFC 8628)
//!   plus AS-metadata discovery (RFC 8414, with fallback to the OIDC
//!   well-known path). Used by both CLIs' `oidc-login` subcommand.
//! - [`ulid`] — Crockford-base32 ULID encode + parse, the display form
//!   for the 16-byte wire ULIDs OHDC carries.
//!
//! Co-locating them in one crate keeps the workspace's CLI-helpers
//! footprint to a single dep edge per consumer rather than fragmenting
//! into a flock of tiny single-purpose crates.

pub mod oidc;
pub mod ulid;
