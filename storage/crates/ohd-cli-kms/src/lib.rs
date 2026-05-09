//! Shared KMS abstraction for OHD CLI vault files.
//!
//! Both `ohd-connect` and `ohd-emergency` ship a small encrypted vault on
//! disk holding `(storage_url, token)` and friends. The two CLIs used to
//! carry near-identical copies of this code; this crate is the dedup.
//!
//! Three backends:
//!
//! - **`Keyring`** — OS secret store (Linux Secret Service / libsecret,
//!   macOS Keychain, Windows Credential Manager) via the `keyring` crate.
//!   Default; best DX.
//! - **`Passphrase`** — derives an AES-GCM key from a user passphrase
//!   via Argon2id. Passphrase is supplied on stdin (TTY prompt) or via
//!   the consumer-defined env var (e.g. `OHD_CONNECT_VAULT_PASSPHRASE`).
//! - **`None`** — passthrough (legacy mode + tests).
//!
//! The on-disk envelope is the same JSON shape regardless of backend, so
//! a future migration tool can re-key without changing the format.
//!
//! ## Per-consumer parameterisation
//!
//! Each CLI passes a [`KmsConfig`] describing its namespace:
//!
//! - `keyring_service` — service name under the OS keyring
//!   (e.g. `"ohd-connect.cli"`).
//! - `env_passphrase_var` — env var consulted before prompting
//!   (e.g. `"OHD_CONNECT_VAULT_PASSPHRASE"`).
//! - `aad` — bytes mixed into AES-GCM as Additional Authenticated Data
//!   (e.g. `b"ohd-connect.vault.v1"`). Different AAD across CLIs means a
//!   vault file from one CLI won't decrypt under the other backend even
//!   if both end up sharing a passphrase.
//! - `prompt_create` / `prompt_open` — user-facing text in the rpassword
//!   prompt fallback.
//!
//! Both CLIs keep the constants colocated next to their other namespacing
//! so package renames stay obvious.

use std::collections::BTreeMap;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use anyhow::{anyhow, Context, Result};
use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use rand_core::{OsRng, RngCore};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

/// Schema version for [`VaultEnvelope`]. Bumped when the JSON shape
/// changes. The Argon2id / AES-GCM parameters are tied to v1; a future
/// v2 would let us migrate to e.g. XChaCha20-Poly1305 without breaking
/// readers.
pub const VAULT_FORMAT_VERSION: u32 = 1;

/// Fixed user under which keyring stores the wrapping key. Tied to
/// VAULT_FORMAT_VERSION so a v2 envelope ships with `vault-key-v2`.
pub const KEYRING_USER: &str = "vault-key-v1";

/// Per-CLI namespace constants. Both consumers (`ohd-connect`,
/// `ohd-emergency`) build one of these and pass it to every backend
/// method.
#[derive(Debug, Clone, Copy)]
pub struct KmsConfig {
    /// Service name registered under the OS keyring
    /// (e.g. `"ohd-connect.cli"`).
    pub keyring_service: &'static str,
    /// Env var name read before falling back to a TTY prompt
    /// (e.g. `"OHD_CONNECT_VAULT_PASSPHRASE"`).
    pub env_passphrase_var: &'static str,
    /// AES-GCM Additional Authenticated Data. Distinct AAD across CLIs
    /// means a vault encrypted under one CLI's passphrase backend won't
    /// authenticate under the other's, even if both happen to use the
    /// same passphrase string.
    pub aad: &'static [u8],
    /// rpassword prompt shown when *creating* a new vault.
    pub prompt_create: &'static str,
    /// rpassword prompt shown when *opening* an existing vault.
    pub prompt_open: &'static str,
}

/// On-disk envelope wrapping an AEAD ciphertext. Each backend emits the
/// same JSON shape (with backend-specific `kms_meta`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultEnvelope {
    pub version: u32,
    pub kms: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ciphertext_b64: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plaintext_b64: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kms_meta: BTreeMap<String, String>,
}

/// One of the supported backends.
pub enum KmsBackend {
    Keyring,
    Passphrase {
        /// Optional explicit passphrase. When absent we read the
        /// consumer's env var, then fall back to a TTY prompt.
        passphrase: Option<SecretString>,
    },
    None,
}

impl KmsBackend {
    pub fn name(&self) -> &'static str {
        match self {
            KmsBackend::Keyring => "keyring",
            KmsBackend::Passphrase { .. } => "passphrase",
            KmsBackend::None => "none",
        }
    }

    /// Construct from a CLI flag value. `"auto"` tries keyring then
    /// falls back to passphrase if the OS keyring is unavailable.
    pub fn from_str_or_auto(value: &str, cfg: &KmsConfig) -> Result<Self> {
        match value {
            "auto" => {
                if probe_keyring(cfg).is_ok() {
                    Ok(KmsBackend::Keyring)
                } else {
                    Ok(KmsBackend::Passphrase { passphrase: None })
                }
            }
            "keyring" => Ok(KmsBackend::Keyring),
            "passphrase" => Ok(KmsBackend::Passphrase { passphrase: None }),
            "none" => Ok(KmsBackend::None),
            other => Err(anyhow!(
                "unknown KMS backend `{other}`; expected auto|keyring|passphrase|none"
            )),
        }
    }

    pub fn encrypt(&self, plaintext: &[u8], cfg: &KmsConfig) -> Result<VaultEnvelope> {
        match self {
            KmsBackend::None => Ok(VaultEnvelope {
                version: VAULT_FORMAT_VERSION,
                kms: "none".into(),
                ciphertext_b64: None,
                plaintext_b64: Some(B64.encode(plaintext)),
                kms_meta: BTreeMap::new(),
            }),
            KmsBackend::Keyring => {
                let key_bytes = keyring_get_or_create_key(cfg)?;
                let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
                let cipher = Aes256Gcm::new(key);
                let mut nonce_bytes = [0u8; 12];
                OsRng.fill_bytes(&mut nonce_bytes);
                let nonce = Nonce::from_slice(&nonce_bytes);
                let ct = cipher
                    .encrypt(
                        nonce,
                        aes_gcm::aead::Payload {
                            msg: plaintext,
                            aad: cfg.aad,
                        },
                    )
                    .map_err(|e| anyhow!("AES-GCM encrypt: {e}"))?;
                let mut meta = BTreeMap::new();
                meta.insert("nonce_b64".into(), B64.encode(nonce_bytes));
                meta.insert("aead".into(), "AES-GCM".into());
                meta.insert("service".into(), cfg.keyring_service.into());
                meta.insert("user".into(), KEYRING_USER.into());
                Ok(VaultEnvelope {
                    version: VAULT_FORMAT_VERSION,
                    kms: "keyring".into(),
                    ciphertext_b64: Some(B64.encode(&ct)),
                    plaintext_b64: None,
                    kms_meta: meta,
                })
            }
            KmsBackend::Passphrase { passphrase } => {
                let passphrase = resolve_passphrase(passphrase, cfg, true)?;
                let mut salt = [0u8; 16];
                OsRng.fill_bytes(&mut salt);
                let key = derive_key_argon2(passphrase.expose_secret(), &salt)?;
                let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
                let mut nonce_bytes = [0u8; 12];
                OsRng.fill_bytes(&mut nonce_bytes);
                let nonce = Nonce::from_slice(&nonce_bytes);
                let ct = cipher
                    .encrypt(
                        nonce,
                        aes_gcm::aead::Payload {
                            msg: plaintext,
                            aad: cfg.aad,
                        },
                    )
                    .map_err(|e| anyhow!("AES-GCM encrypt: {e}"))?;
                let mut meta = BTreeMap::new();
                meta.insert("salt_b64".into(), B64.encode(salt));
                meta.insert("nonce_b64".into(), B64.encode(nonce_bytes));
                meta.insert("kdf".into(), "argon2id".into());
                meta.insert("aead".into(), "AES-GCM".into());
                Ok(VaultEnvelope {
                    version: VAULT_FORMAT_VERSION,
                    kms: "passphrase".into(),
                    ciphertext_b64: Some(B64.encode(&ct)),
                    plaintext_b64: None,
                    kms_meta: meta,
                })
            }
        }
    }

    pub fn decrypt(&self, envelope: &VaultEnvelope, cfg: &KmsConfig) -> Result<Vec<u8>> {
        if envelope.kms != self.name() {
            return Err(anyhow!(
                "vault envelope has kms=`{}`, but selected backend is `{}`",
                envelope.kms,
                self.name()
            ));
        }
        match self {
            KmsBackend::None => {
                let pt_b64 = envelope
                    .plaintext_b64
                    .as_deref()
                    .ok_or_else(|| anyhow!("none-backend envelope missing plaintext_b64"))?;
                B64.decode(pt_b64).map_err(|e| anyhow!("base64: {e}"))
            }
            KmsBackend::Keyring => {
                let ct_b64 = envelope
                    .ciphertext_b64
                    .as_deref()
                    .ok_or_else(|| anyhow!("keyring envelope missing ciphertext_b64"))?;
                let nonce_b64 = envelope
                    .kms_meta
                    .get("nonce_b64")
                    .ok_or_else(|| anyhow!("keyring envelope missing nonce_b64"))?;
                let ct = B64.decode(ct_b64).map_err(|e| anyhow!("ct base64: {e}"))?;
                let nonce_bytes = B64
                    .decode(nonce_b64)
                    .map_err(|e| anyhow!("nonce base64: {e}"))?;
                if nonce_bytes.len() != 12 {
                    return Err(anyhow!("nonce is not 12 bytes"));
                }
                let nonce = Nonce::from_slice(&nonce_bytes);
                let key_bytes = keyring_get_or_create_key(cfg)?;
                let key = Key::<Aes256Gcm>::from_slice(&key_bytes);
                let cipher = Aes256Gcm::new(key);
                cipher
                    .decrypt(
                        nonce,
                        aes_gcm::aead::Payload {
                            msg: &ct,
                            aad: cfg.aad,
                        },
                    )
                    .map_err(|_| anyhow!("AES-GCM tag mismatch (key may have rotated)"))
            }
            KmsBackend::Passphrase { passphrase } => {
                let ct_b64 = envelope
                    .ciphertext_b64
                    .as_deref()
                    .ok_or_else(|| anyhow!("passphrase envelope missing ciphertext_b64"))?;
                let salt_b64 = envelope
                    .kms_meta
                    .get("salt_b64")
                    .ok_or_else(|| anyhow!("passphrase envelope missing salt_b64"))?;
                let nonce_b64 = envelope
                    .kms_meta
                    .get("nonce_b64")
                    .ok_or_else(|| anyhow!("passphrase envelope missing nonce_b64"))?;
                let ct = B64.decode(ct_b64).map_err(|e| anyhow!("ct base64: {e}"))?;
                let salt = B64.decode(salt_b64).map_err(|e| anyhow!("salt base64: {e}"))?;
                let nonce_bytes = B64
                    .decode(nonce_b64)
                    .map_err(|e| anyhow!("nonce base64: {e}"))?;
                let phrase = resolve_passphrase(passphrase, cfg, false)?;
                let key = derive_key_argon2(phrase.expose_secret(), &salt)?;
                let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
                cipher
                    .decrypt(
                        Nonce::from_slice(&nonce_bytes),
                        aes_gcm::aead::Payload {
                            msg: &ct,
                            aad: cfg.aad,
                        },
                    )
                    .map_err(|_| anyhow!("AES-GCM tag mismatch (wrong passphrase or tampered file)"))
            }
        }
    }
}

fn derive_key_argon2(passphrase: &str, salt: &[u8]) -> Result<[u8; 32]> {
    // m_cost in KB, t_cost iterations, parallelism. 64 MiB / 3 / 1 is a
    // defensible interactive-CLI baseline (~150ms on a desktop).
    let params = Params::new(64 * 1024, 3, 1, Some(32))
        .map_err(|e| anyhow!("argon2 params: {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut out)
        .map_err(|e| anyhow!("argon2: {e}"))?;
    Ok(out)
}

fn resolve_passphrase(
    explicit: &Option<SecretString>,
    cfg: &KmsConfig,
    creating: bool,
) -> Result<SecretString> {
    if let Some(p) = explicit {
        return Ok(p.clone());
    }
    if let Ok(p) = std::env::var(cfg.env_passphrase_var) {
        if !p.is_empty() {
            return Ok(SecretString::from(p));
        }
    }
    let prompt = if creating {
        cfg.prompt_create
    } else {
        cfg.prompt_open
    };
    let raw = rpassword::prompt_password(prompt).context("read passphrase from stdin")?;
    Ok(SecretString::from(raw))
}

// ---------------------------------------------------------------------------
// Keyring helpers
// ---------------------------------------------------------------------------

fn probe_keyring(cfg: &KmsConfig) -> Result<()> {
    // Try a get cycle to ensure the backend is reachable. We don't
    // actually persist a probe value — `get_password` succeeds (returning
    // a `NoEntry` error) if the backend is up.
    let entry = keyring::Entry::new(cfg.keyring_service, KEYRING_USER)
        .map_err(|e| anyhow!("keyring entry: {e}"))?;
    match entry.get_password() {
        Ok(_) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(anyhow!("keyring backend unavailable: {e}")),
    }
}

fn keyring_get_or_create_key(cfg: &KmsConfig) -> Result<[u8; 32]> {
    let entry = keyring::Entry::new(cfg.keyring_service, KEYRING_USER)
        .map_err(|e| anyhow!("keyring entry: {e}"))?;
    match entry.get_password() {
        Ok(b64) => {
            let bytes = B64
                .decode(b64.as_bytes())
                .map_err(|e| anyhow!("keyring stored key: bad base64: {e}"))?;
            if bytes.len() != 32 {
                return Err(anyhow!(
                    "keyring stored key has length {}, expected 32",
                    bytes.len()
                ));
            }
            let mut out = [0u8; 32];
            out.copy_from_slice(&bytes);
            Ok(out)
        }
        Err(keyring::Error::NoEntry) => {
            let mut new_key = [0u8; 32];
            OsRng.fill_bytes(&mut new_key);
            entry
                .set_password(&B64.encode(new_key))
                .map_err(|e| anyhow!("keyring set: {e}"))?;
            Ok(new_key)
        }
        Err(e) => Err(anyhow!("keyring get: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal config used by unit tests. Doesn't matter what the names
    /// are — these tests don't touch the OS keyring.
    const TEST_CFG: KmsConfig = KmsConfig {
        keyring_service: "ohd-cli-kms.test",
        env_passphrase_var: "OHD_CLI_KMS_TEST_PASSPHRASE",
        aad: b"ohd-cli-kms.test.v1",
        prompt_create: "test new vault: ",
        prompt_open: "test vault: ",
    };

    #[test]
    fn none_backend_round_trip() {
        let backend = KmsBackend::None;
        let envelope = backend.encrypt(b"hello world", &TEST_CFG).unwrap();
        let decoded = backend.decrypt(&envelope, &TEST_CFG).unwrap();
        assert_eq!(decoded, b"hello world");
    }

    #[test]
    fn passphrase_backend_round_trip() {
        let backend = KmsBackend::Passphrase {
            passphrase: Some(SecretString::from("hunter2".to_string())),
        };
        let envelope = backend.encrypt(b"sensitive", &TEST_CFG).unwrap();
        let decoded = backend.decrypt(&envelope, &TEST_CFG).unwrap();
        assert_eq!(decoded, b"sensitive");
    }

    #[test]
    fn passphrase_wrong_phrase_fails() {
        let enc = KmsBackend::Passphrase {
            passphrase: Some(SecretString::from("right".to_string())),
        };
        let env = enc.encrypt(b"secret", &TEST_CFG).unwrap();
        let dec = KmsBackend::Passphrase {
            passphrase: Some(SecretString::from("wrong".to_string())),
        };
        assert!(dec.decrypt(&env, &TEST_CFG).is_err());
    }

    #[test]
    fn envelope_round_trip_through_json() {
        // The on-disk format is `serde_json::to_string_pretty(envelope)`.
        // Verify a None-backend envelope survives a JSON round-trip.
        let backend = KmsBackend::None;
        let envelope = backend.encrypt(b"toml=payload", &TEST_CFG).unwrap();
        let serialised = serde_json::to_string_pretty(&envelope).unwrap();
        let parsed: VaultEnvelope = serde_json::from_str(&serialised).unwrap();
        let recovered = backend.decrypt(&parsed, &TEST_CFG).unwrap();
        assert_eq!(recovered, b"toml=payload");
    }
}
