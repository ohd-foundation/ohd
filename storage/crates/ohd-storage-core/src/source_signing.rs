//! Source signing for high-trust integrations.
//!
//! Per `spec/docs/components/connect.md` "Source signing": Libre, Dexcom,
//! lab providers, hospital EHRs may sign their submissions with a per-
//! integration key so storage records "this glucose reading was signed by
//! Libre's key X". Protects against leaked-token attackers forging
//! integration writes (the leaked token alone is no longer sufficient — the
//! attacker would also need the integration's signing key).
//!
//! # Algorithms
//!
//! - **Ed25519** (default, `sig_alg = "ed25519"`) — compact, fast, no
//!   parameter pitfalls. Verified via `ed25519-dalek` against the
//!   PEM-encoded SubjectPublicKeyInfo registered in the `signers` table.
//! - **RS256** / **ES256** — for OAuth-aligned integrations that already
//!   issue keys via JWKS-style infrastructure. Verified via the
//!   `jsonwebtoken` workspace dep (`DecodingKey::from_*_pem`).
//!
//! # Canonical encoding
//!
//! The to-be-signed bytes are deterministic CBOR of the fixed shape below.
//! `ciborium`'s primitive serializer is byte-deterministic for these types,
//! so two implementations writing the same field set produce identical
//! bytes. This matches the channel-encryption module's choice (CBOR for
//! consistency across the encryption surface).
//!
//! ```text
//! { "ulid":         <16-byte ULID>,
//!   "timestamp_ms": <i64>,
//!   "event_type":   <text>,
//!   "channels":     [ {"path": <text>, "value": <ChannelScalar>}, … ] }
//! ```
//!
//! Signers compute `signature = sign(canonical_event_bytes(event))`. Storage
//! recomputes `canonical_event_bytes` from the inserted row and verifies the
//! signature against the registered public key. Tampering after the fact
//! (operator rewrites a value) breaks verification on next QueryEvents.
//!
//! # Wire model
//!
//! `EventInput` gains an optional `source_signature` carrying
//! `{sig_alg, signer_kid, signature_bytes}`. When present, `put_events`
//! verifies the signature before commit. On valid signature, the inserted
//! event row gets a paired `event_signatures` row. Verification failure
//! returns `Error::InvalidSignature`.
//!
//! # Operator RPCs
//!
//! - [`register_signer`] — INSERT a new `signers` row.
//! - [`list_signers`] — SELECT all signers (active + revoked).
//! - [`revoke_signer`] — UPDATE `revoked_at_ms` on the row matching `kid`.
//!
//! All three are self-session-only at the wire boundary (caller checked
//! before invoking these helpers).
//!
//! # Threat model deviation
//!
//! Source signing is opt-in per integration. A naked event without a
//! `source_signature` is **not rejected** — the storage still accepts it.
//! What signing buys is verifiability for events the operator (or a leaked
//! token) tries to forge: the operator can't mint a Libre-signed glucose
//! reading without Libre's seckey. UI can render "signed by Libre" badges
//! to surface this to users.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::events::{ChannelScalar, ChannelValue, EventInput};
use crate::ulid::Ulid;
use crate::{Error, Result};

/// One operator-registered signer (an integration's public key).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signer {
    /// Internal rowid.
    pub id: i64,
    /// Operator-assigned key id (e.g. `"libre.eu.2026-01"`).
    pub signer_kid: String,
    /// Human label.
    pub signer_label: String,
    /// `"ed25519"` | `"rs256"` | `"es256"`.
    pub sig_alg: String,
    /// PEM-encoded SubjectPublicKeyInfo.
    pub public_key_pem: String,
    /// Registration timestamp.
    pub registered_at_ms: i64,
    /// Revocation timestamp; `None` = active.
    pub revoked_at_ms: Option<i64>,
}

/// One source signature on an [`EventInput`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceSignature {
    /// Algorithm: `"ed25519"` | `"rs256"` | `"es256"`. Must match the
    /// algorithm the registered signer was registered under.
    pub sig_alg: String,
    /// Signer KID; lookup key into the `signers` table.
    pub signer_kid: String,
    /// Signature bytes.
    pub signature: Vec<u8>,
}

/// Materialized signer info returned alongside a queried event so consumers
/// can render "signed by …" badges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignerInfo {
    /// `signers.signer_kid`.
    pub signer_kid: String,
    /// `signers.signer_label`.
    pub signer_label: String,
    /// `signers.sig_alg`.
    pub sig_alg: String,
    /// Whether the signer was revoked at query time. Existing signed events
    /// are still readable; only new submissions under a revoked kid get
    /// rejected.
    pub revoked: bool,
}

// =============================================================================
// Canonical encoding (deterministic CBOR).
// =============================================================================

/// Wire shape that's CBOR-encoded as the to-be-signed bytes.
///
/// Field names are short (`u`, `t`, `e`, `c`) to keep the canonical form
/// compact; signers replicate the same shape on their side. Channels are
/// sorted by `path` for deterministic encoding (CBOR maps don't have a
/// canonical order on their own; we order by path before serializing).
#[derive(Serialize, Deserialize)]
struct CanonicalEvent<'a> {
    /// 16-byte ULID.
    u: &'a [u8],
    /// Timestamp ms (i64).
    t: i64,
    /// Event type dotted name.
    e: &'a str,
    /// Sorted channels.
    c: Vec<CanonicalChannel<'a>>,
}

#[derive(Serialize, Deserialize)]
struct CanonicalChannel<'a> {
    p: &'a str,
    v: CanonicalScalar,
}

/// Tagged-union mirror of [`ChannelScalar`] for canonical CBOR. Distinct
/// type so the serde tags ("real"/"int"/...) are deterministic.
#[derive(Serialize, Deserialize)]
#[serde(tag = "k", content = "v")]
enum CanonicalScalar {
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

impl From<&ChannelScalar> for CanonicalScalar {
    fn from(s: &ChannelScalar) -> Self {
        match s {
            ChannelScalar::Real { real_value } => CanonicalScalar::Real(*real_value),
            ChannelScalar::Int { int_value } => CanonicalScalar::Int(*int_value),
            ChannelScalar::Bool { bool_value } => CanonicalScalar::Bool(*bool_value),
            ChannelScalar::Text { text_value } => CanonicalScalar::Text(text_value.clone()),
            ChannelScalar::EnumOrdinal { enum_ordinal } => {
                CanonicalScalar::EnumOrdinal(*enum_ordinal)
            }
        }
    }
}

/// Compute canonical CBOR bytes for an `EventInput` paired with the ULID it
/// will be inserted under. Signers must replicate this exact pipeline:
///
/// 1. Reject non-finite floats (`NaN`, `Inf`, `-Inf`) — Codex review #10:
///    different encoders normalize them differently, breaking byte-
///    determinism that signers depend on.
/// 2. Reject duplicate `channel_path` entries — Codex review #11:
///    sorting alone leaves duplicate-path order input-dependent.
/// 3. Sort `channels` by `channel_path` (lex order).
/// 4. Build the `CanonicalEvent { u, t, e, c }` shape.
/// 5. CBOR-encode via ciborium.
pub fn canonical_event_bytes(event: &EventInput, ulid: &Ulid) -> Result<Vec<u8>> {
    // Codex review #10: reject non-finite floats. Different stacks may
    // normalize / reject NaN / Inf / -Inf differently, which breaks the
    // "two implementations produce the same canonical bytes" guarantee
    // the signer pipeline depends on.
    for c in &event.channels {
        if let ChannelScalar::Real { real_value } = &c.value {
            if !real_value.is_finite() {
                return Err(Error::InvalidArgument(
                    "non-finite float in signed event".into(),
                ));
            }
        }
    }
    // Codex review #11: reject duplicate channel paths. Sorting by path
    // doesn't establish a canonical order for ties — two signers given
    // the same input could emit different canonical bytes if the input
    // contained two channels with the same path.
    {
        let mut seen: std::collections::HashSet<&str> =
            std::collections::HashSet::with_capacity(event.channels.len());
        for c in &event.channels {
            if !seen.insert(&c.channel_path) {
                return Err(Error::InvalidArgument("duplicate channel path".into()));
            }
        }
    }
    let mut sorted_channels: Vec<&ChannelValue> = event.channels.iter().collect();
    sorted_channels.sort_by(|a, b| a.channel_path.cmp(&b.channel_path));
    let canonical = CanonicalEvent {
        u: ulid,
        t: event.timestamp_ms,
        e: &event.event_type,
        c: sorted_channels
            .iter()
            .map(|c| CanonicalChannel {
                p: &c.channel_path,
                v: (&c.value).into(),
            })
            .collect(),
    };
    let mut buf = Vec::with_capacity(64 + 32 * sorted_channels.len());
    ciborium::ser::into_writer(&canonical, &mut buf)
        .map_err(|e| Error::Internal(anyhow::anyhow!("CBOR encode canonical event: {e}")))?;
    Ok(buf)
}

// =============================================================================
// Operator registry RPCs.
// =============================================================================

/// Register a signer's public key + algorithm. Self-session-only at the wire
/// boundary; this helper assumes the caller's auth check already passed.
///
/// Returns the inserted [`Signer`].
pub fn register_signer(
    conn: &Connection,
    signer_kid: &str,
    signer_label: &str,
    sig_alg: &str,
    public_key_pem: &str,
) -> Result<Signer> {
    validate_alg(sig_alg)?;
    let now = crate::format::now_ms();
    conn.execute(
        "INSERT INTO signers (signer_kid, signer_label, sig_alg, public_key_pem, registered_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![signer_kid, signer_label, sig_alg, public_key_pem, now],
    )?;
    let id = conn.last_insert_rowid();
    Ok(Signer {
        id,
        signer_kid: signer_kid.to_string(),
        signer_label: signer_label.to_string(),
        sig_alg: sig_alg.to_string(),
        public_key_pem: public_key_pem.to_string(),
        registered_at_ms: now,
        revoked_at_ms: None,
    })
}

/// List all signers (active + revoked). Caller filters as needed.
pub fn list_signers(conn: &Connection) -> Result<Vec<Signer>> {
    let mut stmt = conn.prepare(
        "SELECT id, signer_kid, signer_label, sig_alg, public_key_pem,
                registered_at_ms, revoked_at_ms
           FROM signers ORDER BY registered_at_ms DESC",
    )?;
    let rows = stmt
        .query_map([], |r| {
            Ok(Signer {
                id: r.get(0)?,
                signer_kid: r.get(1)?,
                signer_label: r.get(2)?,
                sig_alg: r.get(3)?,
                public_key_pem: r.get(4)?,
                registered_at_ms: r.get(5)?,
                revoked_at_ms: r.get(6)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

/// Mark a signer as revoked. Existing signed events stay verifiable (the row
/// is still in `signers`); new submissions under this kid are rejected.
pub fn revoke_signer(conn: &Connection, signer_kid: &str) -> Result<i64> {
    let now = crate::format::now_ms();
    let n = conn.execute(
        "UPDATE signers SET revoked_at_ms = ?1
          WHERE signer_kid = ?2 AND revoked_at_ms IS NULL",
        params![now, signer_kid],
    )?;
    if n == 0 {
        return Err(Error::NotFound);
    }
    Ok(now)
}

/// Look up a signer by KID. Returns the row regardless of revocation status
/// (callers gate on `revoked_at_ms` themselves).
pub fn lookup_signer(conn: &Connection, signer_kid: &str) -> Result<Signer> {
    let row: Option<Signer> = conn
        .query_row(
            "SELECT id, signer_kid, signer_label, sig_alg, public_key_pem,
                    registered_at_ms, revoked_at_ms
               FROM signers WHERE signer_kid = ?1",
            params![signer_kid],
            |r| {
                Ok(Signer {
                    id: r.get(0)?,
                    signer_kid: r.get(1)?,
                    signer_label: r.get(2)?,
                    sig_alg: r.get(3)?,
                    public_key_pem: r.get(4)?,
                    registered_at_ms: r.get(5)?,
                    revoked_at_ms: r.get(6)?,
                })
            },
        )
        .optional()?;
    row.ok_or(Error::NotFound)
}

/// Look up the [`SignerInfo`] for an event row, if it has a signature.
/// Used by query paths to decorate the event with `signed_by` metadata.
pub fn signer_info_for_event(conn: &Connection, event_id: i64) -> Result<Option<SignerInfo>> {
    let kid: Option<String> = conn
        .query_row(
            "SELECT signer_kid FROM event_signatures WHERE event_id = ?1",
            params![event_id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(kid) = kid else { return Ok(None) };
    let signer = match lookup_signer(conn, &kid) {
        Ok(s) => s,
        Err(Error::NotFound) => return Ok(None),
        Err(e) => return Err(e),
    };
    Ok(Some(SignerInfo {
        signer_kid: signer.signer_kid,
        signer_label: signer.signer_label,
        sig_alg: signer.sig_alg,
        revoked: signer.revoked_at_ms.is_some(),
    }))
}

// =============================================================================
// Verify-on-insert.
// =============================================================================

/// Verify a [`SourceSignature`] against an [`EventInput`] paired with the
/// ULID the row will be inserted under. Returns Ok on valid signature;
/// `Error::InvalidArgument` on unknown / revoked signer, algorithm
/// mismatch, or signature verify failure.
pub fn verify_signature(
    conn: &Connection,
    event: &EventInput,
    ulid: &Ulid,
    sig: &SourceSignature,
) -> Result<()> {
    validate_alg(&sig.sig_alg)?;
    let signer = lookup_signer(conn, &sig.signer_kid).map_err(|e| match e {
        Error::NotFound => Error::InvalidArgument(format!(
            "INVALID_SIGNATURE: unknown signer_kid {:?}",
            sig.signer_kid
        )),
        other => other,
    })?;
    if signer.revoked_at_ms.is_some() {
        return Err(Error::InvalidArgument(format!(
            "INVALID_SIGNATURE: signer_kid {:?} is revoked",
            sig.signer_kid
        )));
    }
    if signer.sig_alg != sig.sig_alg {
        return Err(Error::InvalidArgument(format!(
            "INVALID_SIGNATURE: sig_alg mismatch (registered={}, supplied={})",
            signer.sig_alg, sig.sig_alg
        )));
    }
    let canonical = canonical_event_bytes(event, ulid)?;
    match sig.sig_alg.as_str() {
        "ed25519" => verify_ed25519(&signer.public_key_pem, &canonical, &sig.signature)?,
        "rs256" | "es256" => verify_jwt_alg(
            &signer.sig_alg,
            &signer.public_key_pem,
            &canonical,
            &sig.signature,
        )?,
        other => {
            return Err(Error::InvalidArgument(format!(
                "INVALID_SIGNATURE: unsupported sig_alg {other}"
            )))
        }
    }
    Ok(())
}

/// Persist the signature row alongside an inserted event. Idempotent on the
/// `(event_id)` PK.
pub fn record_signature(conn: &Connection, event_id: i64, sig: &SourceSignature) -> Result<()> {
    let now = crate::format::now_ms();
    conn.execute(
        "INSERT OR REPLACE INTO event_signatures
            (event_id, sig_alg, signer_kid, signature, signed_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![event_id, sig.sig_alg, sig.signer_kid, sig.signature, now],
    )?;
    Ok(())
}

fn validate_alg(alg: &str) -> Result<()> {
    match alg {
        "ed25519" | "rs256" | "es256" => Ok(()),
        other => Err(Error::InvalidArgument(format!(
            "unsupported sig_alg: {other:?} (expected 'ed25519' | 'rs256' | 'es256')"
        ))),
    }
}

fn verify_ed25519(public_key_pem: &str, msg: &[u8], signature: &[u8]) -> Result<()> {
    use ed25519_dalek::pkcs8::DecodePublicKey;
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let key = VerifyingKey::from_public_key_pem(public_key_pem)
        .map_err(|e| Error::InvalidArgument(format!("INVALID_SIGNATURE: bad ed25519 PEM: {e}")))?;
    if signature.len() != Signature::BYTE_SIZE {
        return Err(Error::InvalidArgument(format!(
            "INVALID_SIGNATURE: ed25519 signature must be {} bytes",
            Signature::BYTE_SIZE
        )));
    }
    let mut sig_bytes = [0u8; Signature::BYTE_SIZE];
    sig_bytes.copy_from_slice(signature);
    let sig = Signature::from_bytes(&sig_bytes);
    key.verify(msg, &sig)
        .map_err(|_| Error::InvalidArgument("INVALID_SIGNATURE: ed25519 verify failed".into()))
}

fn verify_jwt_alg(alg: &str, public_key_pem: &str, msg: &[u8], signature: &[u8]) -> Result<()> {
    // RS256/ES256 verification via jsonwebtoken's primitive `verify` over a
    // detached signature: signers compute `signature = sign(msg)` (no JWT
    // header.payload.signature framing) and supply the raw signature bytes.
    // We base64url-encode the signature for jsonwebtoken's internal verify
    // helper, which expects the JWT-style URL-safe base64.
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    use jsonwebtoken::{Algorithm, DecodingKey};
    let alg_enum = match alg {
        "rs256" => Algorithm::RS256,
        "es256" => Algorithm::ES256,
        _ => unreachable!("validate_alg should have caught"),
    };
    let key = match alg_enum {
        Algorithm::RS256 => DecodingKey::from_rsa_pem(public_key_pem.as_bytes())
            .map_err(|e| Error::InvalidArgument(format!("INVALID_SIGNATURE: bad RSA PEM: {e}")))?,
        Algorithm::ES256 => DecodingKey::from_ec_pem(public_key_pem.as_bytes()).map_err(|e| {
            Error::InvalidArgument(format!("INVALID_SIGNATURE: bad ES256 PEM: {e}"))
        })?,
        _ => unreachable!(),
    };
    let sig_b64 = URL_SAFE_NO_PAD.encode(signature);
    let ok = jsonwebtoken::crypto::verify(&sig_b64, msg, &key, alg_enum).map_err(|e| {
        Error::InvalidArgument(format!("INVALID_SIGNATURE: {alg} verify error: {e}"))
    })?;
    if !ok {
        return Err(Error::InvalidArgument(format!(
            "INVALID_SIGNATURE: {alg} verify failed"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{ChannelScalar, ChannelValue};

    fn evt() -> EventInput {
        EventInput {
            timestamp_ms: 1_700_000_000_000,
            event_type: "std.blood_glucose".into(),
            channels: vec![ChannelValue {
                channel_path: "value".into(),
                value: ChannelScalar::Real { real_value: 5.6 },
            }],
            ..Default::default()
        }
    }

    #[test]
    fn canonical_bytes_are_deterministic() {
        let e = evt();
        let u = [9u8; 16];
        let a = canonical_event_bytes(&e, &u).unwrap();
        let b = canonical_event_bytes(&e, &u).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn canonical_bytes_sort_channels() {
        let mut e = evt();
        e.channels.push(ChannelValue {
            channel_path: "aaa".into(),
            value: ChannelScalar::Int { int_value: 1 },
        });
        let u = [0u8; 16];
        let a = canonical_event_bytes(&e, &u).unwrap();
        // Reorder channels — canonical bytes should still match.
        e.channels.reverse();
        let b = canonical_event_bytes(&e, &u).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn validate_alg_matrix() {
        assert!(validate_alg("ed25519").is_ok());
        assert!(validate_alg("rs256").is_ok());
        assert!(validate_alg("es256").is_ok());
        assert!(validate_alg("hs256").is_err());
        assert!(validate_alg("").is_err());
    }
}
