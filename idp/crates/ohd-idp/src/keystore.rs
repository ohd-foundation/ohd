//! Signing-key rotation.
//!
//! The IdP signs `id_token`s with one **active** RS256 key (the PEM at
//! `keys.signing_key_file`, managed by [`crate::keys::SigningKey`]).
//! Rotation generates a fresh keypair, makes it the active key, and keeps
//! the *previous* key's public material in the JWKS for
//! `keys.rotation_overlap_days` so `id_token`s already minted under the
//! old key still verify.
//!
//! State is two files next to the signing key:
//!
//! - `signing-key.pem` — the active private key (PKCS#8 PEM).
//! - `signing-key.overlap.json` — a list of retired *public* keys, each
//!   with its `kid`, JWK `n`/`e`, and a `retire_after` Unix timestamp.
//!   The verifying side only needs the public key, so retired private
//!   keys are not kept.
//!
//! On startup [`KeyStore::load`] loads the active key and prunes any
//! overlap entry whose `retire_after` has passed. [`KeyStore::rotate`]
//! moves the current active key into the overlap list (retiring it
//! `rotation_overlap_days` from now) and writes a new active key.
//!
//! `/jwks` serves the active key plus every non-expired overlap key.

use crate::jwks::{Jwk, Jwks};
use crate::keys::SigningKey;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A retired signing key kept in the JWKS for the rotation overlap. Only
/// the public material is stored — verification needs nothing more.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlapKey {
    /// The retired key's stable `kid` — matches old `id_token` headers.
    pub kid: String,
    /// Base64url RSA modulus.
    pub n: String,
    /// Base64url RSA public exponent.
    pub e: String,
    /// Unix seconds after which this key is dropped from the JWKS.
    pub retire_after: i64,
}

impl OverlapKey {
    /// The JWK form published at `/jwks`.
    fn to_jwk(&self) -> Jwk {
        Jwk {
            kty: "RSA".to_string(),
            use_: "sig".to_string(),
            alg: "RS256".to_string(),
            kid: self.kid.clone(),
            n: self.n.clone(),
            e: self.e.clone(),
        }
    }
}

/// The IdP's signing-key set: one active key plus retired overlap keys.
#[derive(Clone)]
pub struct KeyStore {
    /// The key new `id_token`s are signed with.
    active: SigningKey,
    /// Retired public keys still inside their overlap window.
    overlap: Vec<OverlapKey>,
    /// Path of the active-key PEM.
    key_path: PathBuf,
    /// Path of the overlap-keys JSON sidecar.
    overlap_path: PathBuf,
}

/// The overlap sidecar filename, derived from the signing-key path:
/// `signing-key.pem` → `signing-key.overlap.json`.
fn overlap_path_for(key_path: &Path) -> PathBuf {
    let stem = key_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("signing-key");
    let file = format!("{stem}.overlap.json");
    match key_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(file),
        _ => PathBuf::from(file),
    }
}

impl KeyStore {
    /// Load the active key (generating it on first launch) and the overlap
    /// list, dropping any overlap entry whose window has closed. The
    /// pruned list is written back so the on-disk state stays tidy.
    pub fn load(key_path: &Path) -> Result<Self> {
        let active = SigningKey::load_or_generate(key_path)?;
        let overlap_path = overlap_path_for(key_path);
        let mut overlap = read_overlap(&overlap_path)?;
        let now = now_unix();
        let before = overlap.len();
        overlap.retain(|k| k.retire_after > now);
        let store = Self {
            active,
            overlap,
            key_path: key_path.to_path_buf(),
            overlap_path,
        };
        if store.overlap.len() != before {
            store.write_overlap()?;
            tracing::info!(
                dropped = before - store.overlap.len(),
                "pruned expired rotation-overlap keys"
            );
        }
        Ok(store)
    }

    /// Build a `KeyStore` from an in-memory key, no persistence — for tests.
    pub fn in_memory(active: SigningKey) -> Self {
        Self {
            active,
            overlap: Vec::new(),
            key_path: PathBuf::new(),
            overlap_path: PathBuf::new(),
        }
    }

    /// The active signing key — what `id_token`s are minted with.
    pub fn active(&self) -> &SigningKey {
        &self.active
    }

    /// The retired overlap keys still inside their window.
    pub fn overlap(&self) -> &[OverlapKey] {
        &self.overlap
    }

    /// Build the `/jwks` document: the active key first, then every
    /// non-expired overlap key. Expired entries are filtered defensively
    /// even though `load`/`rotate` already prune them.
    pub fn jwks(&self) -> Jwks {
        let now = now_unix();
        let mut keys = vec![Jwk::from_signing_key(&self.active)];
        for k in &self.overlap {
            if k.retire_after > now {
                keys.push(k.to_jwk());
            }
        }
        Jwks { keys }
    }

    /// Rotate the signing key: the current active key becomes an overlap
    /// key (retiring `overlap_days` from now), and a freshly generated
    /// keypair becomes active. Persists the new PEM + the overlap sidecar.
    /// Returns `(old_kid, new_kid)`.
    pub fn rotate(&mut self, overlap_days: i64) -> Result<(String, String)> {
        let old_kid = self.active.kid().to_string();
        let retired = OverlapKey {
            kid: old_kid.clone(),
            n: self.active.jwk_modulus(),
            e: self.active.jwk_exponent(),
            retire_after: now_unix() + overlap_days.max(0) * 86_400,
        };

        let new_key = SigningKey::generate().context("generating rotated signing key")?;
        let new_kid = new_key.kid().to_string();
        // Persist the new active key first; only then commit to memory +
        // the overlap sidecar so a crash mid-rotation cannot lose the key
        // we are about to sign with.
        new_key
            .persist(&self.key_path)
            .context("persisting rotated signing key")?;

        // Drop any expired entry while we are here, then prepend the just-
        // retired key so the newest overlap key is first.
        let now = now_unix();
        self.overlap.retain(|k| k.retire_after > now);
        self.overlap.insert(0, retired);
        self.active = new_key;
        self.write_overlap()
            .context("persisting rotation-overlap keys")?;
        Ok((old_kid, new_kid))
    }

    /// Write the overlap list to its JSON sidecar (no-op for an in-memory
    /// store, which has an empty path).
    fn write_overlap(&self) -> Result<()> {
        if self.overlap_path.as_os_str().is_empty() {
            return Ok(());
        }
        let json = serde_json::to_string_pretty(&self.overlap)
            .context("serializing overlap keys")?;
        std::fs::write(&self.overlap_path, json)
            .with_context(|| format!("writing {}", self.overlap_path.display()))?;
        Ok(())
    }
}

/// Read the overlap sidecar; a missing file is an empty list.
fn read_overlap(path: &Path) -> Result<Vec<OverlapKey>> {
    match std::fs::read_to_string(path) {
        Ok(text) => {
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
    }
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_keystore_has_only_the_active_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signing-key.pem");
        let ks = KeyStore::load(&path).unwrap();
        let jwks = ks.jwks();
        assert_eq!(jwks.keys.len(), 1);
        assert_eq!(jwks.keys[0].kid, ks.active().kid());
        assert!(ks.overlap().is_empty());
    }

    #[test]
    fn rotation_keeps_the_old_key_in_the_jwks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signing-key.pem");
        let mut ks = KeyStore::load(&path).unwrap();
        let old_kid = ks.active().kid().to_string();

        let (reported_old, new_kid) = ks.rotate(7).unwrap();
        assert_eq!(reported_old, old_kid);
        assert_ne!(new_kid, old_kid);
        assert_eq!(ks.active().kid(), new_kid);

        // Both keys are in the JWKS during the overlap.
        let kids: Vec<_> = ks.jwks().keys.iter().map(|k| k.kid.clone()).collect();
        assert!(kids.contains(&old_kid));
        assert!(kids.contains(&new_kid));
        assert_eq!(kids.len(), 2);
    }

    #[test]
    fn rotation_persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signing-key.pem");
        let new_kid;
        let old_kid;
        {
            let mut ks = KeyStore::load(&path).unwrap();
            old_kid = ks.active().kid().to_string();
            let (_, nk) = ks.rotate(7).unwrap();
            new_kid = nk;
        }
        // A fresh load reads the rotated PEM + the overlap sidecar.
        let ks = KeyStore::load(&path).unwrap();
        assert_eq!(ks.active().kid(), new_kid);
        let kids: Vec<_> = ks.jwks().keys.iter().map(|k| k.kid.clone()).collect();
        assert!(kids.contains(&old_kid));
        assert_eq!(kids.len(), 2);
    }

    #[test]
    fn expired_overlap_key_is_dropped_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signing-key.pem");
        {
            // Rotate with a zero-day overlap → the retired key expires
            // immediately (retire_after == now).
            let mut ks = KeyStore::load(&path).unwrap();
            ks.rotate(0).unwrap();
        }
        // A reload prunes the already-expired overlap key.
        let ks = KeyStore::load(&path).unwrap();
        assert_eq!(ks.jwks().keys.len(), 1);
        assert!(ks.overlap().is_empty());
    }

    #[test]
    fn overlap_sidecar_path_sits_next_to_the_pem() {
        let p = overlap_path_for(Path::new("/var/lib/ohd-idp/signing-key.pem"));
        assert_eq!(p, Path::new("/var/lib/ohd-idp/signing-key.overlap.json"));
    }
}
