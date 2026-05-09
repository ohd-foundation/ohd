//! Operator-side configuration file.
//!
//! Lives at `$XDG_CONFIG_HOME/ohd-emergency/config.toml` (typically
//! `~/.config/ohd-emergency/config.toml`) with mode 0600. The decrypted
//! payload looks like:
//!
//! ```toml
//! storage_url     = "http://localhost:8443"      # OHDC endpoint (h2c or https+h3)
//! token           = "ohds_..."                   # operator's bearer
//! refresh_token   = "ohdr_..."                   # optional, set by oidc-login
//! access_expires_at_ms = 1700000000000           # optional
//! oidc_issuer     = "https://accounts.example"   # optional
//! oidc_client_id  = "ohd-emergency-cli"          # optional
//! oidc_subject    = "abc123"                     # optional
//! station_label   = "EMS Prague Region"          # free-form
//! authority_cert  = "/etc/ohd-emergency/ca.pem"  # optional path used by `cert info`
//! roster_path     = "/etc/ohd-emergency/roster.toml"  # optional override
//! ```
//!
//! v0.x: the on-disk file is a JSON **vault envelope** wrapping AES-GCM
//! ciphertext by default (see [`crate::kms`]). Mode 0600 is still set on
//! Unix for defence-in-depth.
//!
//! Backwards compat: legacy plaintext-TOML configs still load (we sniff
//! the first byte; envelopes start with `{`, plain TOML starts with a
//! key or comment). New saves always emit the envelope.
//!
//! `ohd-emergency login --storage URL --token ohds_...` writes the file
//! via the configured KMS backend (`--kms-backend auto|keyring|passphrase|none`).
//! Subsequent commands read it; CLI flags (`--storage` / `--token`) always
//! override on a per-invocation basis.
//!
//! Operator-side state (the responder roster) is not stored in `config.toml`
//! — see `roster.rs` for that.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use crate::kms::{KmsBackend, VaultEnvelope, CONFIG as KMS_CONFIG};

/// On-disk shape of `config.toml`. Fields beyond `storage_url` / `token`
/// are optional so a minimal `login` invocation works.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// OHDC endpoint, e.g. `http://localhost:8443` or
    /// `https+h3://localhost:18443`.
    pub storage_url: String,

    /// Operator-side bearer token (`ohds_…`). Issued out-of-band by the
    /// storage server today; `ohd-emergency login --token …` saves it.
    pub token: String,

    /// OIDC refresh token (`ohdr_…`). Set by `oidc-login`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,

    /// Unix-ms when the access token stops being valid (set by oidc-login).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_expires_at_ms: Option<u64>,

    /// OIDC issuer URL the bearer was minted by.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_issuer: Option<String>,

    /// OIDC client_id used at login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_client_id: Option<String>,

    /// OIDC `sub` claim, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oidc_subject: Option<String>,

    /// Free-form station / operator label, surfaced in `cert info` output
    /// and the case-export archive header.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub station_label: Option<String>,

    /// Path to the operator's authority cert (PEM) used by `cert info`.
    /// When unset, `cert info` prints a TBD message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_cert: Option<PathBuf>,

    /// Override for the operator-side roster TOML path. When unset, the
    /// roster lives at `$XDG_DATA_HOME/ohd-emergency/roster.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roster_path: Option<PathBuf>,
}

/// Resolve the config dir. Honours `XDG_CONFIG_HOME` on Linux,
/// `~/Library/Application Support` on macOS, `%APPDATA%` on Windows
/// via `directories`.
pub fn config_dir() -> Result<PathBuf> {
    let proj = directories::ProjectDirs::from("org", "ohd", "ohd-emergency")
        .ok_or_else(|| anyhow!("could not determine config dir (no $HOME?)"))?;
    Ok(proj.config_dir().to_path_buf())
}

pub fn config_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("config.toml"))
}

/// Default operator-side roster location.
pub fn default_roster_path() -> Result<PathBuf> {
    let proj = directories::ProjectDirs::from("org", "ohd", "ohd-emergency")
        .ok_or_else(|| anyhow!("could not determine data dir (no $HOME?)"))?;
    Ok(proj.data_dir().to_path_buf().join("roster.toml"))
}

impl Config {
    /// Read `config.toml`. Returns `None` if the file is missing.
    /// Decrypts via the supplied KMS backend if the on-disk file is a
    /// JSON envelope; legacy plaintext-TOML files still parse for
    /// back-compat.
    pub fn load(kms: &KmsBackend) -> Result<Option<Self>> {
        let path = config_path()?;
        if !path.exists() {
            return Ok(None);
        }
        Self::load_from_with(&path, kms).map(Some)
    }

    /// Read a config from an arbitrary path with the supplied KMS backend.
    /// Used by tests and the main `load` path.
    pub fn load_from_with(path: &Path, kms: &KmsBackend) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?;
        let trimmed = raw.trim_start();
        let toml_payload = if trimmed.starts_with('{') {
            // Envelope — JSON wrapping AES-GCM ciphertext.
            let envelope: VaultEnvelope = serde_json::from_str(&raw)
                .with_context(|| format!("parse vault envelope at {}", path.display()))?;
            let runtime_backend = if envelope.kms != kms.name() {
                KmsBackend::from_str_or_auto(&envelope.kms, &KMS_CONFIG)?
            } else {
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
        let cfg: Config = toml::from_str(&toml_payload)
            .with_context(|| format!("parse {}", path.display()))?;
        Ok(cfg)
    }

    /// Read a plaintext-TOML config from an arbitrary path. Kept for
    /// the legacy tests; prefer [`Config::load_from_with`] in new code.
    pub fn load_from(path: &Path) -> Result<Self> {
        Self::load_from_with(path, &KmsBackend::None)
    }

    /// Write `config.toml` via the chosen KMS backend (default keyring →
    /// passphrase fallback). Mode 0600 on Unix.
    pub fn save(&self, kms: &KmsBackend) -> Result<PathBuf> {
        let dir = config_dir()?;
        fs::create_dir_all(&dir).with_context(|| format!("mkdir {}", dir.display()))?;
        let path = config_path()?;
        let serialized = toml::to_string_pretty(self).context("serialize config")?;
        let envelope = kms.encrypt(serialized.as_bytes(), &KMS_CONFIG)?;
        let json = serde_json::to_string_pretty(&envelope)
            .context("serialize vault envelope")?;
        fs::write(&path, &json).with_context(|| format!("write {}", path.display()))?;
        chmod_0600(&path)?;
        Ok(path)
    }

    /// Resolve the (storage_url, token) pair from CLI overrides + config
    /// file. Errors if neither source supplies a value.
    pub fn resolve_storage(
        cli_storage: Option<&str>,
        cli_token: Option<&str>,
        kms: &KmsBackend,
    ) -> Result<(String, String)> {
        let from_file = Self::load(kms)?;
        let storage_url = cli_storage
            .map(|s| s.to_string())
            .or_else(|| from_file.as_ref().map(|c| c.storage_url.clone()))
            .ok_or_else(|| {
                anyhow!(
                    "no storage URL (run `ohd-emergency login --storage URL --token TOKEN` \
                     or `ohd-emergency oidc-login --issuer ...`, or pass --storage)"
                )
            })?;
        let token = cli_token
            .map(|s| s.to_string())
            .or_else(|| from_file.as_ref().map(|c| c.token.clone()))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "no token (run `ohd-emergency login --storage URL --token TOKEN` \
                     or `ohd-emergency oidc-login --issuer ...`, or pass --token)"
                )
            })?;
        Ok((storage_url, token))
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

// ---- tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn round_trip_minimal_plaintext_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let cfg = Config {
            storage_url: "http://localhost:8443".into(),
            token: "ohds_TESTTOKEN".into(),
            ..Default::default()
        };
        let s = toml::to_string_pretty(&cfg).unwrap();
        std::fs::write(&path, s).unwrap();

        // Plaintext-TOML still loads via load_from / load_from_with(None).
        let parsed = Config::load_from(&path).unwrap();
        assert_eq!(parsed.storage_url, "http://localhost:8443");
        assert_eq!(parsed.token, "ohds_TESTTOKEN");
        assert!(parsed.station_label.is_none());
        assert!(parsed.authority_cert.is_none());
    }

    #[test]
    fn round_trip_full_plaintext_toml() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let raw = r#"
storage_url     = "https+h3://localhost:18443"
token           = "ohds_FULL"
station_label   = "EMS Prague"
authority_cert  = "/etc/ohd-emergency/ca.pem"
"#;
        std::fs::write(&path, raw).unwrap();
        let parsed = Config::load_from(&path).unwrap();
        assert_eq!(parsed.storage_url, "https+h3://localhost:18443");
        assert_eq!(parsed.station_label.as_deref(), Some("EMS Prague"));
        assert_eq!(
            parsed.authority_cert.as_deref(),
            Some(std::path::Path::new("/etc/ohd-emergency/ca.pem"))
        );
    }

    #[test]
    fn missing_required_fields_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "station_label = \"x\"\n").unwrap();
        let err = Config::load_from(&path).unwrap_err();
        let msg = format!("{err:#}");
        assert!(msg.contains("storage_url") || msg.contains("missing"));
    }

    /// Encrypted-on-disk round-trip via the passphrase backend.
    /// Mirrors the connect/cli credential vault test shape.
    #[test]
    fn round_trip_through_envelope_passphrase() {
        use secrecy::SecretString;
        let cfg = Config {
            storage_url: "https+h3://localhost:18443".into(),
            token: "ohds_VAULTED".into(),
            station_label: Some("EMS Prague — vaulted".into()),
            oidc_issuer: Some("https://accounts.example".into()),
            oidc_client_id: Some("ohd-emergency-cli".into()),
            ..Default::default()
        };
        let kms = KmsBackend::Passphrase {
            passphrase: Some(SecretString::from("hunter2".to_string())),
        };
        // Manually encrypt + decrypt without touching the user's
        // real config dir — Config::save would write to ~/.config/...
        let serialized = toml::to_string_pretty(&cfg).unwrap();
        let envelope = kms.encrypt(serialized.as_bytes(), &KMS_CONFIG).unwrap();
        let plaintext = kms.decrypt(&envelope, &KMS_CONFIG).unwrap();
        let s = String::from_utf8(plaintext).unwrap();
        let parsed: Config = toml::from_str(&s).unwrap();
        assert_eq!(parsed.storage_url, "https+h3://localhost:18443");
        assert_eq!(parsed.token, "ohds_VAULTED");
        assert_eq!(parsed.station_label.as_deref(), Some("EMS Prague — vaulted"));
        assert_eq!(parsed.oidc_issuer.as_deref(), Some("https://accounts.example"));
    }

    /// Sniff: `load_from_with` correctly demuxes envelope JSON from
    /// plaintext TOML on disk.
    #[test]
    fn load_from_with_demuxes_envelope_vs_toml() {
        let dir = tempdir().unwrap();

        // 1) Write a plaintext-TOML config.
        let toml_path = dir.path().join("plain.toml");
        std::fs::write(
            &toml_path,
            r#"storage_url = "http://localhost:8443"
token = "ohds_PLAIN"
"#,
        )
        .unwrap();
        let parsed_plain = Config::load_from_with(&toml_path, &KmsBackend::None).unwrap();
        assert_eq!(parsed_plain.token, "ohds_PLAIN");

        // 2) Write an envelope-encoded config (None backend; base64'd
        // plaintext) and verify it loads through the same entry point.
        let env_path = dir.path().join("env.toml");
        let cfg = Config {
            storage_url: "http://localhost:8443".into(),
            token: "ohds_VAULTED".into(),
            ..Default::default()
        };
        let kms = KmsBackend::None;
        let serialized = toml::to_string_pretty(&cfg).unwrap();
        let envelope = kms.encrypt(serialized.as_bytes(), &KMS_CONFIG).unwrap();
        let json = serde_json::to_string_pretty(&envelope).unwrap();
        std::fs::write(&env_path, json).unwrap();
        let parsed_env = Config::load_from_with(&env_path, &KmsBackend::None).unwrap();
        assert_eq!(parsed_env.token, "ohds_VAULTED");
    }
}
