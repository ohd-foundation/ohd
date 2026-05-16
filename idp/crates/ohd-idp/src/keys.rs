//! RS256 signing-key management.
//!
//! On first launch a 2048-bit RSA keypair is generated and persisted as
//! PKCS#8 PEM at `keys.signing_key_file`. On every subsequent launch the
//! same PEM is loaded, so the derived `kid` — and therefore every
//! `id_token` ever signed under it — stays stable across restarts.
//!
//! The `kid` is the base64url-encoded SHA-256 of the DER-encoded public
//! key (an RFC 7638-style thumbprint over the SPKI bytes). It is
//! deterministic from the key material, so no extra state is stored.
//!
//! Rotation (a later phase) generates a new key; both public keys then
//! coexist in the JWKS for `rotation_overlap_days`. The types here are
//! shaped so an additional key slot can be added without reshaping —
//! see [`crate::jwks`].

use anyhow::{Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::EncodingKey;
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use rsa::traits::PublicKeyParts;
use rsa::{RsaPrivateKey, RsaPublicKey};
use sha2::{Digest, Sha256};
use std::path::Path;

/// RSA modulus size for generated signing keys.
const KEY_BITS: usize = 2048;

/// A loaded RS256 signing key: the private key for minting `id_token`s
/// and the derived public material for the JWKS.
#[derive(Clone)]
pub struct SigningKey {
    private: RsaPrivateKey,
    public: RsaPublicKey,
    /// Stable key id, derived from the public key.
    kid: String,
}

impl SigningKey {
    /// Load the signing key from `path`, generating + persisting a new
    /// keypair there if the file does not yet exist.
    ///
    /// The parent directory is created if missing. The PEM is written
    /// with `0600` permissions on Unix.
    pub fn load_or_generate(path: &Path) -> Result<Self> {
        if path.exists() {
            Self::load(path)
        } else {
            let key = Self::generate()?;
            key.persist(path)?;
            tracing::info!(file = %path.display(), kid = %key.kid, "generated new RS256 signing key");
            Ok(key)
        }
    }

    /// Generate a fresh in-memory keypair (not persisted).
    pub fn generate() -> Result<Self> {
        let mut rng = rand::thread_rng();
        let private = RsaPrivateKey::new(&mut rng, KEY_BITS)
            .context("generating RSA signing keypair")?;
        let public = RsaPublicKey::from(&private);
        let kid = derive_kid(&public)?;
        Ok(Self { private, public, kid })
    }

    /// Load a keypair from a PKCS#8 PEM file.
    pub fn load(path: &Path) -> Result<Self> {
        let pem = std::fs::read_to_string(path)
            .with_context(|| format!("reading signing key {}", path.display()))?;
        Self::from_pkcs8_pem(&pem)
    }

    /// Parse a keypair from PKCS#8 PEM text.
    pub fn from_pkcs8_pem(pem: &str) -> Result<Self> {
        let private = RsaPrivateKey::from_pkcs8_pem(pem)
            .context("parsing PKCS#8 PEM signing key")?;
        let public = RsaPublicKey::from(&private);
        let kid = derive_kid(&public)?;
        Ok(Self { private, public, kid })
    }

    /// Write the private key to `path` as PKCS#8 PEM, creating the parent
    /// directory and restricting permissions to the owner.
    pub fn persist(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("creating key directory {}", dir.display()))?;
        }
        let pem = self
            .private
            .to_pkcs8_pem(LineEnding::LF)
            .context("encoding signing key to PKCS#8 PEM")?;
        std::fs::write(path, pem.as_bytes())
            .with_context(|| format!("writing signing key {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .with_context(|| format!("restricting permissions on {}", path.display()))?;
        }
        Ok(())
    }

    /// The stable key id published in the JWKS and the `id_token` header.
    pub fn kid(&self) -> &str {
        &self.kid
    }

    /// The RSA public key, for JWKS publication.
    pub fn public_key(&self) -> &RsaPublicKey {
        &self.public
    }

    /// The base64url RSA modulus (`n`) for a JWK.
    pub fn jwk_modulus(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.public.n().to_bytes_be())
    }

    /// The base64url RSA public exponent (`e`) for a JWK.
    pub fn jwk_exponent(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.public.e().to_bytes_be())
    }

    /// A `jsonwebtoken` [`EncodingKey`] over the RSA private key, for
    /// signing `id_token`s with RS256. Built from the PKCS#1 DER of the
    /// private key — the form `jsonwebtoken` accepts.
    pub fn encoding_key(&self) -> Result<EncodingKey> {
        let der = self
            .private
            .to_pkcs1_der()
            .context("encoding signing key to PKCS#1 DER")?;
        Ok(EncodingKey::from_rsa_der(der.as_bytes()))
    }
}

/// Derive a stable `kid`: base64url(SHA-256(DER-encoded SPKI)).
fn derive_kid(public: &RsaPublicKey) -> Result<String> {
    let der = public
        .to_public_key_der()
        .context("encoding public key to DER")?;
    let digest = Sha256::digest(der.as_bytes());
    Ok(URL_SAFE_NO_PAD.encode(digest))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_key_has_nonempty_kid_and_jwk_params() {
        let key = SigningKey::generate().unwrap();
        assert!(!key.kid().is_empty());
        assert!(!key.jwk_modulus().is_empty());
        // RSA public exponent is conventionally 65537 → "AQAB".
        assert_eq!(key.jwk_exponent(), "AQAB");
    }

    #[test]
    fn pem_round_trip_preserves_kid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("signing-key.pem");
        let first = SigningKey::load_or_generate(&path).unwrap();
        // A second load reads the persisted PEM — same kid.
        let second = SigningKey::load_or_generate(&path).unwrap();
        assert_eq!(first.kid(), second.kid());
        assert_eq!(first.jwk_modulus(), second.jwk_modulus());
    }
}
