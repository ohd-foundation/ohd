//! At-rest sealing for the credentials CORD must persist — share grant
//! tokens and bring-your-own model keys. AES-256-GCM under the deployment
//! `data_key`; the SQLite file alone is inert without it.
//!
//! Wire layout of a sealed blob: `base64( nonce[12] || ciphertext+tag )`.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use anyhow::{anyhow, bail};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use rand::RngCore;

/// Seal `plaintext` with `key`. Returns a base64 string safe to store as
/// SQLite TEXT.
pub fn seal(key: &[u8; 32], plaintext: &[u8]) -> String {
    let cipher = Aes256Gcm::new(key.into());
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let ct = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext)
        .expect("AES-GCM encryption is infallible for valid keys");
    let mut out = Vec::with_capacity(12 + ct.len());
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ct);
    STANDARD.encode(out)
}

/// Reverse of [`seal`]. Errors on a wrong key, truncation, or tampering.
pub fn unseal(key: &[u8; 32], sealed: &str) -> anyhow::Result<Vec<u8>> {
    let raw = STANDARD
        .decode(sealed)
        .map_err(|e| anyhow!("sealed blob is not base64: {e}"))?;
    if raw.len() < 12 + 16 {
        bail!("sealed blob too short");
    }
    let (nonce_bytes, ct) = raw.split_at(12);
    let cipher = Aes256Gcm::new(key.into());
    cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|_| anyhow!("decryption failed — wrong data key or corrupt blob"))
}

/// Seal a string, returning the base64 blob.
pub fn seal_str(key: &[u8; 32], s: &str) -> String {
    seal(key, s.as_bytes())
}

/// Unseal to a UTF-8 string.
pub fn unseal_str(key: &[u8; 32], sealed: &str) -> anyhow::Result<String> {
    let bytes = unseal(key, sealed)?;
    String::from_utf8(bytes).map_err(|e| anyhow!("unsealed bytes are not UTF-8: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let key = [7u8; 32];
        let sealed = seal_str(&key, "ohdg_secrettoken");
        assert_eq!(unseal_str(&key, &sealed).unwrap(), "ohdg_secrettoken");
    }

    #[test]
    fn wrong_key_fails() {
        let sealed = seal_str(&[1u8; 32], "x");
        assert!(unseal_str(&[2u8; 32], &sealed).is_err());
    }

    #[test]
    fn nonce_is_random_so_ciphertext_differs() {
        let key = [3u8; 32];
        assert_ne!(seal_str(&key, "same"), seal_str(&key, "same"));
    }
}
