//! OHD Storage core library.
//!
//! Owns the on-disk format (SQLite + SQLCipher), the channel registry,
//! grants, audit, sync, encryption-at-rest, and the in-process implementation
//! of the OHDC service surface.
//!
//! # Module map
//!
//! - [`format`] — file open/create, SQLCipher wiring, migration runner.
//! - [`registry`] — event-type / channel registry + alias resolution.
//! - [`events`] — events / event_channels / event_samples DML helpers.
//! - [`grants`] — grant rows + read/write rule tables, scope intersection.
//! - [`cases`] — cases + case_filters scaffolding (full resolver is v1.x).
//! - [`pending`] — `pending_events` approval queue helpers.
//! - [`audit`] — append-only audit log per RPC.
//! - [`sync`] — placeholder; bidirectional event-log replay (v1.x).
//! - [`encryption`] — key hierarchy (K_envelope / K_class), wrap / unwrap, rotation.
//! - [`channel_encryption`] — value-level AEAD pipeline for sensitive channels.
//! - [`source_signing`] — Ed25519/RS256/ES256 verification for high-trust integration writes.
//! - [`auth`] — token resolution to one of three auth profiles.
//! - [`ulid`] — mint / parse / encode / split honouring the pre-1970 clamp.
//! - [`ohdc`] — in-process implementation of the OHDC service surface.
//! - [`storage`] — top-level [`Storage`] handle wrapping a connection.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod attachments;
pub mod audit;
pub mod auth;
pub mod cases;
pub mod channel_encryption;
pub mod device_tokens;
pub mod emergency_config;
pub mod encryption;
pub mod error;
pub mod events;
pub mod format;
pub mod grants;
pub mod identities;
pub mod invites;
pub mod notification_config;
pub mod ohdc;
pub mod pending;
pub mod pending_queries;
pub mod push_registrations;
pub mod registry;
pub mod sample_codec;
pub mod samples;
pub mod sessions;
pub mod source_signing;
pub mod storage;
pub mod sync;
pub mod ulid;

pub use error::{Error, Result};
pub use storage::{Storage, StorageConfig};

/// On-disk format version this build understands.
///
/// Mirrors `_meta.format_version`. See `spec/storage-format.md`.
pub const FORMAT_VERSION: &str = "1.0";

/// Wire protocol version this build implements.
///
/// Mirrors `Health.protocol_version` in OHDC. See `spec/ohdc-protocol.md`.
pub const PROTOCOL_VERSION: &str = "ohdc.v0";

/// Build version string (component release version, not protocol version).
pub const STORAGE_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Convenience health summary used by the binary's `health` subcommand and by
/// `OhdcService.Health`.
#[derive(Debug, Clone)]
pub struct HealthSummary {
    /// `"ok" | "degraded" | "down"`.
    pub status: &'static str,
    /// Human-readable build/version string for the storage binary.
    pub server_version: &'static str,
    /// OHDC protocol version this build implements.
    pub protocol_version: &'static str,
}

impl HealthSummary {
    /// Returns the standard "ok" health summary.
    pub fn ok() -> Self {
        Self {
            status: "ok",
            server_version: STORAGE_VERSION,
            protocol_version: PROTOCOL_VERSION,
        }
    }
}
