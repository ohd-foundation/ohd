//! OAuth/OIDC signing-key lifecycle: generate, encrypt-at-rest, list as JWKS,
//! sign id_tokens.
//!
//! v0 ships RS256 keys (2048-bit RSA) generated lazily on the first
//! [`mint_id_token`] call. The private key is encrypted under the storage's
//! [`EnvelopeKey`] when one is available (= production), plaintext otherwise
//! (= no-cipher testing path; the caller's threat model is "tests only").
//!
//! ## Rotation
//!
//! Calling [`rotate_active_key`] retires every `rotated_at_ms IS NULL` row,
//! generates a fresh keypair, and inserts it as the new active row. The old
//! public JWK stays in the JWKS so already-issued id_tokens still verify
//! until they expire.
//!
//! ## Encryption-at-rest layout
//!
//! When `wrap_alg = 'aes-256-gcm'`:
//! ```text
//!   nonce: 12 bytes (column `nonce`)
//!   private_key_pem: ciphertext + 16-byte AEAD tag (column `private_key_pem`)
//! ```
//! AAD = `b"ohd.v0.oauth_signing_key:" || kid_bytes`. This binds the wrap to
//! the row's `kid` so an operator can't shuffle rows around without breaking
//! the AEAD tag.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{AeadCore, Aes256Gcm, Key, Nonce};
use jsonwebtoken::jwk::{
    AlgorithmParameters, CommonParameters, Jwk, JwkSet, KeyAlgorithm, PublicKeyUse,
    RSAKeyParameters, RSAKeyType,
};
use jsonwebtoken::{encode as jwt_encode, Algorithm, EncodingKey, Header};
use ohd_storage_core::encryption::EnvelopeKey;
use ohd_storage_core::storage::Storage;
use ohd_storage_core::{Error, Result};
use rand::rngs::OsRng;
use rsa::pkcs1::EncodeRsaPrivateKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPrivateKey;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

const RSA_BITS: usize = 2048;
const AAD_PREFIX: &[u8] = b"ohd.v0.oauth_signing_key:";
const NONCE_LEN: usize = 12;

/// id_token claims (OIDC Core §2). Minimal: `iss`, `sub`, `aud`, `exp`,
/// `iat`, `auth_time`. Extra app-specific claims are out of scope for v0.
#[derive(Serialize)]
struct IdTokenClaims<'a> {
    iss: &'a str,
    sub: String,
    aud: &'a str,
    exp: i64,
    iat: i64,
    auth_time: i64,
}

/// Mint a signed id_token. Generates the active keypair if none exists.
pub fn mint_id_token(
    storage: &Storage,
    issuer: &str,
    audience: &str,
    user_ulid: [u8; 16],
    now_ms: i64,
    ttl_ms: i64,
) -> Result<String> {
    let active = ensure_active_key(storage)?;
    let claims = IdTokenClaims {
        iss: issuer.trim_end_matches('/'),
        sub: ohd_storage_core::ulid::to_crockford(&user_ulid),
        aud: audience,
        exp: (now_ms + ttl_ms) / 1000,
        iat: now_ms / 1000,
        auth_time: now_ms / 1000,
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(active.kid.clone());
    let key = EncodingKey::from_rsa_pem(active.private_key_pem.as_bytes())
        .map_err(|e| Error::Internal(anyhow::anyhow!("EncodingKey::from_rsa_pem: {e}")))?;
    jwt_encode(&header, &claims, &key)
        .map_err(|e| Error::Internal(anyhow::anyhow!("jwt encode: {e}")))
}

/// Return the JWK set of every signing key (active + rotated) so consumers
/// can verify both freshly-minted and not-yet-expired older id_tokens.
pub fn list_active_jwks(storage: &Storage) -> Result<JwkSet> {
    // Ensure at least one key exists so /jwks.json never returns an empty set
    // when the daemon was just started with `--oauth-issuer` but hasn't yet
    // minted a token.
    let _ = ensure_active_key(storage)?;
    let rows: Vec<String> = storage.with_conn(|conn| {
        let mut stmt =
            conn.prepare("SELECT public_jwk_json FROM oauth_signing_keys ORDER BY id ASC")?;
        let mut iter = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        while let Some(row) = iter.next() {
            out.push(row?);
        }
        Ok(out)
    })?;
    let mut keys = Vec::with_capacity(rows.len());
    for json in rows {
        let jwk: Jwk = serde_json::from_str(&json)
            .map_err(|e| Error::Internal(anyhow::anyhow!("malformed stored JWK: {e}")))?;
        keys.push(jwk);
    }
    Ok(JwkSet { keys })
}

/// Forcibly retire every active key + mint a fresh one. Old public JWKs stay
/// in the JWKS so previously-issued id_tokens still verify.
#[allow(dead_code)]
pub fn rotate_active_key(storage: &Storage) -> Result<String> {
    let now = ohd_storage_core::format::now_ms();
    storage.with_conn(|conn| {
        conn.execute(
            "UPDATE oauth_signing_keys SET rotated_at_ms = ?1 WHERE rotated_at_ms IS NULL",
            params![now],
        )
        .map_err(Error::from)
    })?;
    let new = generate_and_persist(storage)?;
    Ok(new.kid)
}

/// In-memory active-key bundle.
struct ActiveKey {
    kid: String,
    private_key_pem: String,
}

fn ensure_active_key(storage: &Storage) -> Result<ActiveKey> {
    if let Some(k) = read_active(storage)? {
        return Ok(k);
    }
    generate_and_persist(storage)
}

fn read_active(storage: &Storage) -> Result<Option<ActiveKey>> {
    let envelope = storage.envelope_key().cloned();
    let row: Option<(String, Vec<u8>, Option<String>, Option<Vec<u8>>)> =
        storage.with_conn(|conn| {
            conn.query_row(
                "SELECT kid, private_key_pem, wrap_alg, nonce
               FROM oauth_signing_keys
              WHERE rotated_at_ms IS NULL
              ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .map_err(Error::from)
        })?;
    let Some((kid, blob, wrap_alg, nonce)) = row else {
        return Ok(None);
    };
    let pem = match (wrap_alg.as_deref(), nonce, envelope.as_ref()) {
        (None, _, _) => String::from_utf8(blob)
            .map_err(|e| Error::Internal(anyhow::anyhow!("stored PEM not UTF-8: {e}")))?,
        (Some("aes-256-gcm"), Some(nonce_bytes), Some(env)) => {
            let plaintext = aes_unwrap(env, &kid, &nonce_bytes, &blob)?;
            String::from_utf8(plaintext)
                .map_err(|e| Error::Internal(anyhow::anyhow!("decrypted PEM not UTF-8: {e}")))?
        }
        (Some("aes-256-gcm"), _, None) => {
            return Err(Error::Internal(anyhow::anyhow!(
                "stored OAuth signing key is encrypted but storage has no envelope key"
            )));
        }
        (Some("aes-256-gcm"), None, _) => {
            return Err(Error::Internal(anyhow::anyhow!(
                "stored OAuth signing key wrap_alg = aes-256-gcm but nonce is NULL"
            )));
        }
        (Some(other), _, _) => {
            return Err(Error::Internal(anyhow::anyhow!(
                "unknown wrap_alg: {other:?}"
            )));
        }
    };
    Ok(Some(ActiveKey {
        kid,
        private_key_pem: pem,
    }))
}

fn generate_and_persist(storage: &Storage) -> Result<ActiveKey> {
    let mut rng = rand::thread_rng();
    let private_key = RsaPrivateKey::new(&mut rng, RSA_BITS)
        .map_err(|e| Error::Internal(anyhow::anyhow!("RSA generate: {e}")))?;
    let public_key = private_key.to_public_key();
    let pem = private_key
        .to_pkcs1_pem(rsa::pkcs8::LineEnding::LF)
        .map_err(|e| Error::Internal(anyhow::anyhow!("RSA pkcs1 PEM: {e}")))?
        .to_string();
    let kid = new_kid();
    use base64::Engine;
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let n_bytes = public_key.n().to_bytes_be();
    let e_bytes = public_key.e().to_bytes_be();
    let jwk = Jwk {
        common: CommonParameters {
            public_key_use: Some(PublicKeyUse::Signature),
            key_algorithm: Some(KeyAlgorithm::RS256),
            key_id: Some(kid.clone()),
            ..Default::default()
        },
        algorithm: AlgorithmParameters::RSA(RSAKeyParameters {
            key_type: RSAKeyType::RSA,
            n: b64.encode(&n_bytes),
            e: b64.encode(&e_bytes),
        }),
    };
    let public_json = serde_json::to_string(&jwk)
        .map_err(|e| Error::Internal(anyhow::anyhow!("JWK json: {e}")))?;

    let envelope = storage.envelope_key().cloned();
    let (blob, wrap_alg, nonce_col): (Vec<u8>, Option<&str>, Option<Vec<u8>>) = match envelope {
        Some(env) => {
            let (nonce, ciphertext) = aes_wrap(&env, &kid, pem.as_bytes())?;
            (ciphertext, Some("aes-256-gcm"), Some(nonce))
        }
        None => (pem.as_bytes().to_vec(), None, None),
    };

    let now = ohd_storage_core::format::now_ms();
    storage.with_conn(|conn| {
        conn.execute(
            "INSERT INTO oauth_signing_keys
                (kid, alg, private_key_pem, public_jwk_json, wrap_alg, nonce, created_at_ms)
             VALUES (?1, 'RS256', ?2, ?3, ?4, ?5, ?6)",
            params![kid, blob, public_json, wrap_alg, nonce_col, now],
        )
        .map_err(Error::from)
    })?;
    Ok(ActiveKey {
        kid,
        private_key_pem: pem,
    })
}

fn new_kid() -> String {
    use base64::Engine;
    use rand::RngCore;
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn aes_wrap(envelope: &EnvelopeKey, kid: &str, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(envelope.as_bytes()));
    let nonce_arr = Aes256Gcm::generate_nonce(&mut OsRng);
    let mut aad = Vec::with_capacity(AAD_PREFIX.len() + kid.len());
    aad.extend_from_slice(AAD_PREFIX);
    aad.extend_from_slice(kid.as_bytes());
    let ciphertext = cipher
        .encrypt(
            &nonce_arr,
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| Error::Internal(anyhow::anyhow!("AES-GCM wrap failed")))?;
    Ok((nonce_arr.to_vec(), ciphertext))
}

fn aes_unwrap(
    envelope: &EnvelopeKey,
    kid: &str,
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    if nonce.len() != NONCE_LEN {
        return Err(Error::Internal(anyhow::anyhow!(
            "stored AES-GCM nonce wrong length: {}",
            nonce.len()
        )));
    }
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(envelope.as_bytes()));
    let n: &Nonce<<Aes256Gcm as AeadCore>::NonceSize> = Nonce::from_slice(nonce);
    let mut aad = Vec::with_capacity(AAD_PREFIX.len() + kid.len());
    aad.extend_from_slice(AAD_PREFIX);
    aad.extend_from_slice(kid.as_bytes());
    cipher
        .decrypt(
            n,
            Payload {
                msg: ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| Error::DecryptionFailed)
}
