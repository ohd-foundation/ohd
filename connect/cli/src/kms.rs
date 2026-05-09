//! Per-CLI KMS namespace.
//!
//! The vault crypto + backend dispatch lives in `ohd-cli-kms` (shared
//! with `../../emergency/cli/src/kms.rs`). This thin file just pins the
//! per-CLI namespace constants — keyring service name, env var, AAD,
//! prompt strings — and re-exports the shared types so call sites read
//! `crate::kms::{KmsBackend, VaultEnvelope, CONFIG}` unchanged.
//!
//! See `storage/crates/ohd-cli-kms/src/lib.rs` for the backend
//! implementation + the rationale for each parameter.

pub use ohd_cli_kms::{KmsBackend, KmsConfig, VaultEnvelope};

/// Namespace constants used by every keyring + passphrase call from
/// inside the connect CLI.
pub const CONFIG: KmsConfig = KmsConfig {
    keyring_service: "ohd-connect.cli",
    env_passphrase_var: "OHD_CONNECT_VAULT_PASSPHRASE",
    aad: b"ohd-connect.vault.v1",
    prompt_create: "OHD Connect new vault passphrase: ",
    prompt_open: "OHD Connect vault passphrase: ",
};
