//! Per-channel value-level encryption.
//!
//! This module is the value-side counterpart to [`crate::encryption`]: it
//! handles the AEAD encryption of individual `event_channels.value_*` payloads
//! when the channel's `sensitivity_class` is in the encrypted-classes set.
//! Key management — derivation, wrapping, rotation — lives in
//! [`crate::encryption`]; this module only knows about ciphertexts and CBOR
//! envelopes.
//!
//! # Wire format of an encrypted blob
//!
//! ```text
//! +-------------------------------------------------------------------------+
//! | nonce (24 bytes, XChaCha20-Poly1305 XNONCE)                             |
//! +-------------------------------------------------------------------------+
//! | ciphertext (CBOR-encoded ChannelScalar) || Poly1305 tag (16 bytes)      |
//! +-------------------------------------------------------------------------+
//! ```
//!
//! The `key_id` (the `class_key_history.id` row that wraps the DEK used) is
//! stored separately in `event_channels.encryption_key_id` rather than in the
//! blob — that keeps the row decryptable without parsing the blob and lets
//! SQL JOINs against the key history table work for audit / rotation reports.
//!
//! Single code path: XChaCha20-Poly1305 with the wide AAD spec'd in the
//! Codex review hardening (binds `(channel_path, event_ulid, key_id)`). The
//! legacy AES-256-GCM single-shot V1 path was removed when the encryption
//! codebase was flattened — see STATUS.md "Encryption flattened to V2-only".
//!
//! # CBOR codec
//!
//! `ChannelScalar` is serialized via `ciborium`, which produces a compact
//! self-describing tagged-union encoding. Per the v1 design choice, CBOR is
//! preferred over JSON because:
//!
//! - The ciphertext row is heavier than a plaintext value column (an extra
//!   24+16 bytes for nonce + tag); CBOR keeps the *envelope* tight.
//! - CBOR encodes f64 NaN / Inf without lossy-string tricks.
//! - Round-trip is byte-deterministic on the same input (`ciborium` writes
//!   the canonical form for primitives), which matters for the conformance
//!   harness's "encrypt the same value twice with the same nonce → identical
//!   bytes" assertion.
//!
//! See `spec/encryption.md` "End-to-end channel encryption" for design context.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{AeadCore, Key as ChaKey, XChaCha20Poly1305, XNonce};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};

use crate::encryption::{ClassKey, DEK_LEN, XNONCE_LEN};
use crate::events::ChannelScalar;
use crate::ulid::Ulid;
use crate::{Error, Result};

/// On-disk encrypted-blob layout. Stored verbatim in
/// `event_channels.value_blob`.
///
/// Layout: `[24-byte XChaCha20-Poly1305 nonce][ciphertext + 16-byte tag]`.
#[derive(Debug, Clone)]
pub struct EncryptedBlob {
    /// 24-byte XChaCha20-Poly1305 nonce.
    pub nonce: Vec<u8>,
    /// Ciphertext concatenated with the 16-byte AEAD tag.
    pub ciphertext: Vec<u8>,
}

impl EncryptedBlob {
    /// Serialize the blob to its on-disk byte form: `[nonce(24)][ct+tag]`.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.nonce.len() + self.ciphertext.len());
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.ciphertext);
        out
    }

    /// Parse the on-disk byte form. Returns [`Error::DecryptionFailed`]
    /// when the blob is too short to contain a 24-byte nonce + 16-byte tag.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < XNONCE_LEN + 16 {
            return Err(Error::DecryptionFailed);
        }
        Ok(Self {
            nonce: bytes[..XNONCE_LEN].to_vec(),
            ciphertext: bytes[XNONCE_LEN..].to_vec(),
        })
    }
}

/// AAD: `"ohd.v0.ch:" || channel_path || "|evt:" || event_ulid
/// || "|key:" || encryption_key_id`.
///
/// Codex review #2: a narrow AAD over only the channel path left
/// `(value_blob, encryption_key_id)` swappable across events with the same
/// channel path. Binding `event_ulid` + `key_id` makes any row-level swap
/// fail AEAD verify on the next read.
fn channel_aad(channel_path: &str, event_ulid: &Ulid, key_id: i64) -> Vec<u8> {
    let mut v = Vec::with_capacity(b"ohd.v0.ch:".len() + channel_path.len() + 16 + 32);
    v.extend_from_slice(b"ohd.v0.ch:");
    v.extend_from_slice(channel_path.as_bytes());
    v.extend_from_slice(b"|evt:");
    v.extend_from_slice(event_ulid);
    v.extend_from_slice(b"|key:");
    v.extend_from_slice(key_id.to_le_bytes().as_ref());
    v
}

/// Tagged-union form of [`ChannelScalar`] used as the CBOR-codec target.
///
/// We can't directly CBOR-encode `ChannelScalar` because its serde derive uses
/// `#[serde(untagged)]` for compatibility with the on-the-wire JSON shape
/// (which omits a tag and disambiguates by which `*_value` field is present).
/// Untagged is fine for JSON because the field names disambiguate, but CBOR
/// needs an explicit tag to round-trip booleans (which would otherwise
/// collide with int via the `value_int` route).
///
/// This enum is the explicit-tag mirror; we convert to/from `ChannelScalar`
/// at the encoder/decoder boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "k", content = "v")]
enum CborScalar {
    #[serde(rename = "real")]
    Real(f64),
    #[serde(rename = "int")]
    Int(i64),
    #[serde(rename = "bool")]
    Bool(bool),
    #[serde(rename = "text")]
    Text(String),
    #[serde(rename = "enum")]
    EnumOrdinal(i32),
}

impl From<&ChannelScalar> for CborScalar {
    fn from(s: &ChannelScalar) -> Self {
        match s {
            ChannelScalar::Real { real_value } => CborScalar::Real(*real_value),
            ChannelScalar::Int { int_value } => CborScalar::Int(*int_value),
            ChannelScalar::Bool { bool_value } => CborScalar::Bool(*bool_value),
            ChannelScalar::Text { text_value } => CborScalar::Text(text_value.clone()),
            ChannelScalar::EnumOrdinal { enum_ordinal } => CborScalar::EnumOrdinal(*enum_ordinal),
        }
    }
}

impl From<CborScalar> for ChannelScalar {
    fn from(c: CborScalar) -> Self {
        match c {
            CborScalar::Real(real_value) => ChannelScalar::Real { real_value },
            CborScalar::Int(int_value) => ChannelScalar::Int { int_value },
            CborScalar::Bool(bool_value) => ChannelScalar::Bool { bool_value },
            CborScalar::Text(text_value) => ChannelScalar::Text { text_value },
            CborScalar::EnumOrdinal(enum_ordinal) => ChannelScalar::EnumOrdinal { enum_ordinal },
        }
    }
}

/// Encrypt a channel value under the supplied `K_class` with
/// XChaCha20-Poly1305 and the wide AAD.
///
/// Pipeline:
/// 1. CBOR-encode the [`ChannelScalar`] (tagged form).
/// 2. XChaCha20-Poly1305 encrypt with a fresh 24-byte CSPRNG nonce; AAD
///    binds `(channel_path, event_ulid, key_id)`.
/// 3. Pack `(nonce, ciphertext+tag)` into an [`EncryptedBlob`].
///
/// Codex review findings #1 + #2:
/// - #1 (nonce-collision birthday bound): XChaCha20-Poly1305's 192-bit nonce
///   makes random-nonce collisions astronomically unlikely at any practical
///   write volume.
/// - #2 (replay/swap of `(value_blob, encryption_key_id)`): the AAD binds
///   the event ULID and key id so an operator copying the blob bytes onto
///   a different event's row gets a decrypt failure.
pub fn encrypt_channel_value(
    channel_path: &str,
    value: &ChannelScalar,
    class_key: &ClassKey,
    event_ulid: &Ulid,
    key_id: i64,
) -> Result<EncryptedBlob> {
    let cbor = serialize_cbor(value)?;
    if class_key.as_bytes().len() != DEK_LEN {
        return Err(Error::Internal(anyhow::anyhow!(
            "ClassKey must be 32 bytes for XChaCha20-Poly1305"
        )));
    }
    let cipher = XChaCha20Poly1305::new(ChaKey::from_slice(class_key.as_bytes()));
    let nonce_arr = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let aad = channel_aad(channel_path, event_ulid, key_id);
    let ciphertext = cipher
        .encrypt(
            &nonce_arr,
            Payload {
                msg: &cbor,
                aad: &aad,
            },
        )
        .map_err(|_| {
            Error::Internal(anyhow::anyhow!("XChaCha20-Poly1305 channel encrypt failed"))
        })?;
    Ok(EncryptedBlob {
        nonce: nonce_arr.as_slice().to_vec(),
        ciphertext,
    })
}

/// Decrypt an encrypted channel value.
///
/// `event_ulid` + `key_id` are part of the AAD; the caller fetches them from
/// the `event_channels` row alongside `value_blob`. Tag verification fails
/// if any of `(channel_path, event_ulid, key_id)` has been tampered with at
/// the row level.
pub fn decrypt_channel_value(
    channel_path: &str,
    blob: &EncryptedBlob,
    class_key: &ClassKey,
    event_ulid: &Ulid,
    key_id: i64,
) -> Result<ChannelScalar> {
    if class_key.as_bytes().len() != DEK_LEN {
        return Err(Error::DecryptionFailed);
    }
    if blob.nonce.len() != XNONCE_LEN {
        return Err(Error::DecryptionFailed);
    }
    let cipher = XChaCha20Poly1305::new(ChaKey::from_slice(class_key.as_bytes()));
    let nonce = XNonce::from_slice(&blob.nonce);
    let aad = channel_aad(channel_path, event_ulid, key_id);
    let plaintext = cipher
        .decrypt(
            nonce,
            Payload {
                msg: &blob.ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| Error::DecryptionFailed)?;
    deserialize_cbor(&plaintext)
}

fn serialize_cbor(value: &ChannelScalar) -> Result<Vec<u8>> {
    let cbor = CborScalar::from(value);
    let mut buf = Vec::with_capacity(32);
    ciborium::ser::into_writer(&cbor, &mut buf)
        .map_err(|e| Error::Internal(anyhow::anyhow!("CBOR encode failed: {e}")))?;
    Ok(buf)
}

fn deserialize_cbor(bytes: &[u8]) -> Result<ChannelScalar> {
    let cbor: CborScalar = ciborium::de::from_reader(bytes).map_err(|_| Error::DecryptionFailed)?;
    Ok(ChannelScalar::from(cbor))
}

/// Marker returned in place of a decrypted value when a grant/token doesn't
/// hold the wrap material to decrypt a particular encrypted class.
///
/// The wire form is a `text_value` with the literal contents
/// `"<encrypted: $sensitivity_class>"` so a UI renders it gracefully without
/// special-casing. Per spec, the row is still returned (so the grantee sees
/// "this datapoint exists, you don't have access") rather than silently
/// dropped — that's the difference between a sensitivity-class deny (silent
/// drop) and an encryption-without-wrap-material case (visible redaction).
pub fn redacted_marker(sensitivity_class: &str) -> ChannelScalar {
    ChannelScalar::Text {
        text_value: format!("<encrypted: {sensitivity_class}>"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key() -> ClassKey {
        ClassKey::from_bytes([0xAA; DEK_LEN])
    }

    fn ulid() -> Ulid {
        [9u8; 16]
    }

    #[test]
    fn round_trip_real() {
        let v = ChannelScalar::Real { real_value: 6.4 };
        let blob = encrypt_channel_value("value", &v, &key(), &ulid(), 1).unwrap();
        let out = decrypt_channel_value("value", &blob, &key(), &ulid(), 1).unwrap();
        match out {
            ChannelScalar::Real { real_value } => assert_eq!(real_value, 6.4),
            other => panic!("expected real, got {:?}", other),
        }
    }

    #[test]
    fn round_trip_int() {
        let v = ChannelScalar::Int { int_value: 42 };
        let blob = encrypt_channel_value("count", &v, &key(), &ulid(), 7).unwrap();
        let out = decrypt_channel_value("count", &blob, &key(), &ulid(), 7).unwrap();
        match out {
            ChannelScalar::Int { int_value } => assert_eq!(int_value, 42),
            other => panic!("expected int, got {:?}", other),
        }
    }

    #[test]
    fn round_trip_bool() {
        let v = ChannelScalar::Bool { bool_value: true };
        let blob = encrypt_channel_value("flag", &v, &key(), &ulid(), 1).unwrap();
        let out = decrypt_channel_value("flag", &blob, &key(), &ulid(), 1).unwrap();
        match out {
            ChannelScalar::Bool { bool_value } => assert!(bool_value),
            other => panic!("expected bool, got {:?}", other),
        }
    }

    #[test]
    fn round_trip_text() {
        let v = ChannelScalar::Text {
            text_value: "private notes about my therapy session".into(),
        };
        let blob = encrypt_channel_value("note", &v, &key(), &ulid(), 1).unwrap();
        let out = decrypt_channel_value("note", &blob, &key(), &ulid(), 1).unwrap();
        match out {
            ChannelScalar::Text { text_value } => {
                assert_eq!(text_value, "private notes about my therapy session")
            }
            other => panic!("expected text, got {:?}", other),
        }
    }

    #[test]
    fn round_trip_enum() {
        let v = ChannelScalar::EnumOrdinal { enum_ordinal: 3 };
        let blob = encrypt_channel_value("severity", &v, &key(), &ulid(), 1).unwrap();
        let out = decrypt_channel_value("severity", &blob, &key(), &ulid(), 1).unwrap();
        match out {
            ChannelScalar::EnumOrdinal { enum_ordinal } => assert_eq!(enum_ordinal, 3),
            other => panic!("expected enum, got {:?}", other),
        }
    }

    #[test]
    fn wrong_key_fails() {
        let v = ChannelScalar::Real { real_value: 6.4 };
        let blob = encrypt_channel_value("value", &v, &key(), &ulid(), 1).unwrap();
        let wrong = ClassKey::from_bytes([0xBB; DEK_LEN]);
        let result = decrypt_channel_value("value", &blob, &wrong, &ulid(), 1);
        assert!(matches!(result, Err(Error::DecryptionFailed)));
    }

    #[test]
    fn channel_path_aad_binding() {
        let v = ChannelScalar::Real { real_value: 6.4 };
        let blob = encrypt_channel_value("value", &v, &key(), &ulid(), 1).unwrap();
        let result = decrypt_channel_value("other_path", &blob, &key(), &ulid(), 1);
        assert!(matches!(result, Err(Error::DecryptionFailed)));
    }

    #[test]
    fn event_ulid_aad_binding() {
        // Codex review #2: encrypting under one event ULID, decrypting under
        // another, must fail (the AAD binds the ULID).
        let v = ChannelScalar::Real { real_value: 6.4 };
        let u_a = [1u8; 16];
        let u_b = [2u8; 16];
        let blob = encrypt_channel_value("value", &v, &key(), &u_a, 1).unwrap();
        let result = decrypt_channel_value("value", &blob, &key(), &u_b, 1);
        assert!(matches!(result, Err(Error::DecryptionFailed)));
    }

    #[test]
    fn key_id_aad_binding() {
        // Codex review #2: tampering with the encryption_key_id column on the
        // row breaks the AEAD verify.
        let v = ChannelScalar::Real { real_value: 6.4 };
        let blob = encrypt_channel_value("value", &v, &key(), &ulid(), 7).unwrap();
        let result = decrypt_channel_value("value", &blob, &key(), &ulid(), 8);
        assert!(matches!(result, Err(Error::DecryptionFailed)));
    }

    #[test]
    fn blob_to_from_bytes_round_trip() {
        let v = ChannelScalar::Int { int_value: 9001 };
        let blob = encrypt_channel_value("count", &v, &key(), &ulid(), 1).unwrap();
        let bytes = blob.to_bytes();
        let parsed = EncryptedBlob::from_bytes(&bytes).unwrap();
        let out = decrypt_channel_value("count", &parsed, &key(), &ulid(), 1).unwrap();
        match out {
            ChannelScalar::Int { int_value } => assert_eq!(int_value, 9001),
            other => panic!("expected int, got {:?}", other),
        }
    }

    #[test]
    fn short_blob_rejected() {
        // 39 bytes: 24 nonce + 15 = under the AEAD-tag minimum of 16.
        let bytes = vec![0u8; 39];
        let result = EncryptedBlob::from_bytes(&bytes);
        assert!(matches!(result, Err(Error::DecryptionFailed)));
    }

    #[test]
    fn redacted_marker_round_trip() {
        let m = redacted_marker("mental_health");
        match m {
            ChannelScalar::Text { text_value } => {
                assert_eq!(text_value, "<encrypted: mental_health>")
            }
            _ => panic!("expected text marker"),
        }
    }

    #[test]
    fn xchacha20_24_byte_nonce() {
        // Codex review #1: the value-side AEAD nonce is 192 bits.
        let v = ChannelScalar::Real { real_value: 1.0 };
        let blob = encrypt_channel_value("value", &v, &key(), &ulid(), 1).unwrap();
        assert_eq!(blob.nonce.len(), XNONCE_LEN);
    }

    #[test]
    fn nonce_uniqueness_under_same_class_key() {
        // Codex review #1: 1000 messages encrypted under the same K_class —
        // every nonce must be distinct. Trivially true for the 192-bit
        // XChaCha20 nonce, but this catches a regression to AES-GCM with
        // random nonces in the future.
        use std::collections::HashSet;
        let v = ChannelScalar::Real { real_value: 1.0 };
        let mut seen = HashSet::new();
        for i in 0..1000 {
            let u = [i as u8; 16];
            let blob = encrypt_channel_value("value", &v, &key(), &u, 1).unwrap();
            assert!(
                seen.insert(blob.nonce.clone()),
                "duplicate nonce at iteration {i}"
            );
        }
    }
}
