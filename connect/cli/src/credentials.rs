//! Credentials file support.
//!
//! On first run, `ohd-connect login --storage URL --token TOKEN` writes a
//! TOML payload to `$XDG_CONFIG_HOME/ohd-connect/credentials.toml`
//! (typically `~/.config/ohd-connect/credentials.toml`). v0.x: the file
//! is encrypted at rest via the KMS abstraction in [`crate::kms`] —
//! the on-disk file is now a JSON envelope wrapping AES-GCM ciphertext
//! by default. Mode 0600 is still set on Unix for defence-in-depth.
//!
//! Backwards compat: legacy plaintext-TOML credentials still load
//! (we sniff the first byte; envelopes start with `{`, plain TOML
//! starts with a key or comment). New saves always emit the envelope.
//!
//! On-disk shape (decrypted) — TOML:
//! ```toml
//! storage_url     = "https://ohd.cloud.example"
//! token           = "ohds_..."             # legacy access-token field
//! refresh_token   = "ohdr_..."             # optional, set by oidc-login
//! access_expires_at_ms = 1700000000000
//! oidc_issuer     = "https://accounts.example"
//! oidc_client_id  = "ohd-connect-cli"
//! oidc_subject    = "abc123"
//! ```

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::kms::{KmsBackend, VaultEnvelope, CONFIG as KMS_CONFIG};

/// On-disk shape of the credentials payload (TOML-serializable).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Credentials {
    pub storage_url: String,
    pub token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_expires_at_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_issuer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_client_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_subject: Option<String>,
}

/// Default config dir. Honours `XDG_CONFIG_HOME` on Linux, `~/Library/
/// Application Support` on macOS, `%APPDATA%` on Windows via `directories`.
pub fn config_dir() -> Result<PathBuf> {
    let proj = directories::ProjectDirs::from("org", "ohd", "ohd-connect")
        .ok_or_else(|| anyhow!("could not determine config dir (no $HOME?)"))?;
    Ok(proj.config_dir().to_path_buf())
}

pub fn credentials_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("credentials.toml"))
}

impl Credentials {
    /// Read the credentials file. Returns `None` if the file is missing.
    pub fn load(kms: &KmsBackend) -> Result<Option<Self>> {
        let path = credentials_path()?;
        if !path.exists() {
            return Ok(None);
        }
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("read {}", path.display()))?;
        let trimmed = raw.trim_start();
        let toml_payload = if trimmed.starts_with('{') {
            // Envelope — JSON wrapping AES-GCM ciphertext.
            let envelope: VaultEnvelope = serde_json::from_str(&raw)
                .with_context(|| format!("parse vault envelope at {}", path.display()))?;
            // The selected backend has to match the envelope; re-init
            // if the envelope says something different from `kms`.
            let runtime_backend = if envelope.kms != kms.name() {
                KmsBackend::from_str_or_auto(&envelope.kms, &KMS_CONFIG)?
            } else {
                // Cheap clone: the variant is just a discriminant + an Option.
                // We avoid Clone on KmsBackend to keep `SecretString` honest;
                // construct a fresh one from the same name.
                KmsBackend::from_str_or_auto(kms.name(), &KMS_CONFIG)?
            };
            let plaintext = runtime_backend
                .decrypt(&envelope, &KMS_CONFIG)
                .with_context(|| format!("decrypt {}", path.display()))?;
            String::from_utf8(plaintext)
                .with_context(|| "decrypted payload was not UTF-8")?
        } else {
            raw
        };
        let creds: Credentials = toml::from_str(&toml_payload)
            .with_context(|| format!("parse credentials.toml at {}", path.display()))?;
        Ok(Some(creds))
    }

    /// Write the credentials file. Encrypts via the chosen KMS backend
    /// (default keyring → passphrase fallback). Mode 0600 on Unix.
    pub fn save(&self, kms: &KmsBackend) -> Result<PathBuf> {
        let dir = config_dir()?;
        fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
        let path = credentials_path()?;
        let serialized = toml::to_string_pretty(self).context("serialize credentials")?;
        let envelope = kms.encrypt(serialized.as_bytes(), &KMS_CONFIG)?;
        let json = serde_json::to_string_pretty(&envelope)
            .context("serialize vault envelope")?;
        fs::write(&path, &json).with_context(|| format!("write {}", path.display()))?;
        chmod_0600(&path)?;
        Ok(path)
    }
}

#[cfg(unix)]
fn chmod_0600(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = fs::metadata(path)?.permissions();
    perms.set_mode(0o600);
    fs::set_permissions(path, perms)
        .with_context(|| format!("chmod 0600 {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn chmod_0600(_path: &Path) -> Result<()> {
    Ok(())
}

/// Resolve the (storage_url, token) pair from CLI overrides + credentials
/// file. Errors if neither source supplies a value.
pub fn resolve(
    cli_storage: Option<&str>,
    cli_token: Option<&str>,
    kms: &KmsBackend,
) -> Result<(String, String)> {
    let from_file = Credentials::load(kms)?;
    let storage_url = cli_storage
        .map(|s| s.to_string())
        .or_else(|| from_file.as_ref().map(|c| c.storage_url.clone()))
        .ok_or_else(|| {
            anyhow!(
                "no storage URL (run `ohd-connect login --storage URL --token TOKEN` \
                 or `ohd-connect oidc-login --issuer ...`, or pass --storage)"
            )
        })?;
    let token = cli_token
        .map(|s| s.to_string())
        .or_else(|| from_file.as_ref().map(|c| c.token.clone()))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow!(
                "no token (run `ohd-connect login --storage URL --token TOKEN` \
                 or `ohd-connect oidc-login --issuer ...`, or pass --token)"
            )
        })?;
    Ok((storage_url, token))
}
