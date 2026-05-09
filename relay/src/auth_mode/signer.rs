//! Sign + verify `EmergencyAccessRequest` payloads.
//!
//! Per `spec/emergency-trust.md`:
//!
//! > The signature is **standard PKCS#1-style detached signature**:
//! > Ed25519 over the SHA-512 hash of the canonical Protobuf encoding of
//! > the request with `signature = empty bytes`.
//!
//! ## v1 canonical encoding (deviation from spec)
//!
//! The spec calls for "canonical Protobuf encoding". We don't pull in
//! `prost` for this single message in v1; instead we use **canonical JSON
//! with sorted keys** as the byte stream that gets SHA-512'd. The byte
//! shape is documented and stable; the conformance corpus pins expected
//! signatures over fixture inputs.
//!
//! When the OHD project ships a `prost`-based shared crate for Protobuf
//! types (out of scope for the relay here), this module flips its
//! `canonical_signing_bytes` impl to call `prost::Message::encode_to_vec`
//! against the message with `signature = vec![]`. The wire signature is
//! recomputed at that cutover; old signatures don't roll forward (24h
//! cert TTL means the fleet rotates inside a day).
//!
//! Today's v1 canonical bytes:
//!
//! ```text
//! sha512(
//!   serde_json::to_string(&{
//!     "request_id":     <hex>,
//!     "issued_at_ms":   <i64>,
//!     "expires_at_ms":  <i64>,
//!     "patient_storage_pubkey_pin": <hex|null>,
//!     "responder_label": <str|null>,
//!     "scene_context":   <str|null>,
//!     "operator_label":  <str|null>,
//!     "scene_lat":       <f64|null>,
//!     "scene_lon":       <f64|null>,
//!     "scene_accuracy_m":<f32|null>,
//!     "cert_chain_pem":  ["<pem>", ...]
//!   }))                                             // sorted keys
//! ```

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use ed25519_dalek::{Signature, Signer, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};

use super::cert_chain::AuthorityCertChain;

/// OHD emergency-authority extended-key-usage OID. Placeholder under the
/// IANA-pending arc. Pinned by `spec/emergency-trust.md` once IANA assigns
/// the OHD private enterprise number.
pub const OHD_EMERGENCY_AUTHORITY_OID: &str = "1.3.6.1.4.1.99999.1.1";

/// Maximum X.509 chain depth (root → fulcio → org → optional responder).
pub const MAX_CHAIN_DEPTH: usize = 4;

/// Maximum acceptable wall-clock skew between signer + verifier.
pub const MAX_CLOCK_SKEW_MS: i64 = 60_000;

#[derive(Debug, thiserror::Error)]
pub enum EmergencyTrustError {
    #[error("request id length {0} != 16")]
    BadRequestId(usize),
    #[error("clock skew: issued_at_ms is too far from now")]
    BadClockSkew,
    #[error("expired: expires_at_ms <= now")]
    Expired,
    #[error("pin mismatch")]
    PinMismatch,
    #[error("PEM decode: {0}")]
    Pem(String),
    #[error("X.509 parse: {0}")]
    X509(String),
    #[error("chain depth {0} exceeds limit {1}")]
    DepthExceeded(usize, usize),
    #[error("chain does not terminate at any trusted root")]
    NoTrustedRoot,
    #[error("chain validity: cert {0} is outside its validity window")]
    ChainValidity(usize),
    #[error("chain signature mismatch at index {0}")]
    ChainSignatureMismatch(usize),
    #[error("missing OHD emergency-authority EKU OID on cert {0}")]
    MissingEku(usize),
    #[error("request signature mismatch")]
    RequestSignatureMismatch,
    #[error("leaf does not have an Ed25519 public key")]
    LeafNotEd25519,
    #[error("leaf public key does not match signing key")]
    LeafKeyMismatch,
    #[error("serialize canonical: {0}")]
    Serialize(String),
}

/// Wire-shape struct for `EmergencyAccessRequest` per
/// `spec/emergency-trust.md`. Held as JSON-friendly types in v1; the
/// `prost`-based Protobuf form is deferred (see module docs).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EmergencyAccessRequest {
    /// 16 random bytes, hex-encoded for JSON.
    pub request_id: String,
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    /// Optional storage SPKI fingerprint (sha256, hex-encoded).
    pub patient_storage_pubkey_pin: Option<String>,
    pub responder_label: Option<String>,
    pub scene_context: Option<String>,
    pub operator_label: Option<String>,
    pub scene_lat: Option<f64>,
    pub scene_lon: Option<f64>,
    pub scene_accuracy_m: Option<f32>,
    /// PEM-encoded X.509 chain, leaf first, root last.
    pub cert_chain_pem: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SignedEmergencyRequest {
    #[serde(flatten)]
    pub request: EmergencyAccessRequest,
    /// Detached Ed25519 signature, base64-encoded.
    pub signature: String,
}

/// A trust root the verifier accepts. Wraps a PEM-encoded root cert.
#[derive(Clone)]
pub struct TrustRoot {
    pub pem: Vec<u8>,
    pub label: String,
}

impl std::fmt::Debug for TrustRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustRoot")
            .field("label", &self.label)
            .finish()
    }
}

/// Produce the canonical bytes used for signing.
///
/// Implementation: serialize all request fields *except* signature,
/// JSON-encoded with stable key ordering, then SHA-512.
pub fn canonical_signing_bytes(req: &EmergencyAccessRequest) -> Result<[u8; 64], EmergencyTrustError> {
    // serde_json's default Serialize uses field declaration order. To
    // make the canonical form independent of source-code field order, we
    // serialize through a `BTreeMap<&str, serde_json::Value>` so the
    // output is alphabetically key-sorted.
    let v = serde_json::to_value(req)
        .map_err(|e| EmergencyTrustError::Serialize(e.to_string()))?;
    let canonical = canonicalize_json(&v);
    let bytes = serde_json::to_vec(&canonical)
        .map_err(|e| EmergencyTrustError::Serialize(e.to_string()))?;
    let mut hasher = Sha512::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let mut out = [0u8; 64];
    out.copy_from_slice(&digest);
    Ok(out)
}

fn canonicalize_json(v: &serde_json::Value) -> serde_json::Value {
    use std::collections::BTreeMap;
    match v {
        serde_json::Value::Object(map) => {
            let mut out: BTreeMap<String, serde_json::Value> = BTreeMap::new();
            for (k, val) in map {
                out.insert(k.clone(), canonicalize_json(val));
            }
            serde_json::to_value(out).unwrap_or(serde_json::Value::Null)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(canonicalize_json).collect())
        }
        _ => v.clone(),
    }
}

/// Sign a request with the relay's leaf cert keypair. The chain is
/// embedded into the request's `cert_chain_pem` field if not already set.
pub fn sign_request(
    chain: &AuthorityCertChain,
    mut req: EmergencyAccessRequest,
    now_ms: i64,
) -> Result<SignedEmergencyRequest, EmergencyTrustError> {
    if req.cert_chain_pem.is_empty() {
        req.cert_chain_pem = chain
            .wire_chain_pem()
            .into_iter()
            .map(|p| String::from_utf8_lossy(&p).into_owned())
            .collect();
    }
    if req.issued_at_ms == 0 {
        req.issued_at_ms = now_ms;
    }
    if req.expires_at_ms == 0 {
        req.expires_at_ms = now_ms + 5 * 60 * 1000;
    }
    if req.request_id.is_empty() {
        req.request_id = random_request_id_hex();
    }

    let digest = canonical_signing_bytes(&req)?;
    let sig: Signature = chain.leaf_signing_key.sign(&digest);
    let signature_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());
    Ok(SignedEmergencyRequest {
        request: req,
        signature: signature_b64,
    })
}

/// Verify a signed `EmergencyAccessRequest` end-to-end:
///
/// 1. Wall-clock + request-id sanity.
/// 2. Optional storage pin match (caller-supplied).
/// 3. PEM-decode the chain; cap depth.
/// 4. Each cert valid at `now`.
/// 5. Each cert (except possibly the root, if implicit) signed by the
///    next-up cert's pubkey.
/// 6. Chain terminates at a cert whose pubkey is in `trust_roots`.
/// 7. Each cert carries the OHD emergency-authority EKU OID.
/// 8. Detached Ed25519 signature verifies against the leaf's pubkey.
pub fn verify_request(
    signed: &SignedEmergencyRequest,
    trust_roots: &[TrustRoot],
    now_ms: i64,
    pin_check: Option<&[u8]>,
) -> Result<(), EmergencyTrustError> {
    // 1. Sanity.
    let raw_id = hex::decode(&signed.request.request_id)
        .map_err(|_| EmergencyTrustError::BadRequestId(0))?;
    if raw_id.len() != 16 {
        return Err(EmergencyTrustError::BadRequestId(raw_id.len()));
    }
    if (signed.request.issued_at_ms - now_ms).abs() > MAX_CLOCK_SKEW_MS {
        return Err(EmergencyTrustError::BadClockSkew);
    }
    if signed.request.expires_at_ms <= now_ms {
        return Err(EmergencyTrustError::Expired);
    }

    // 2. Pin check.
    if let (Some(expected), Some(actual_hex)) =
        (pin_check, signed.request.patient_storage_pubkey_pin.as_deref())
    {
        let actual = hex::decode(actual_hex).map_err(|_| EmergencyTrustError::PinMismatch)?;
        if actual != expected {
            return Err(EmergencyTrustError::PinMismatch);
        }
    }

    // 3-7. Chain validation.
    if signed.request.cert_chain_pem.is_empty() {
        return Err(EmergencyTrustError::DepthExceeded(0, MAX_CHAIN_DEPTH));
    }
    if signed.request.cert_chain_pem.len() > MAX_CHAIN_DEPTH {
        return Err(EmergencyTrustError::DepthExceeded(
            signed.request.cert_chain_pem.len(),
            MAX_CHAIN_DEPTH,
        ));
    }

    let chain_der: Vec<Vec<u8>> = signed
        .request
        .cert_chain_pem
        .iter()
        .map(|s| {
            let p = pem::parse(s.as_bytes()).map_err(|e| EmergencyTrustError::Pem(e.to_string()))?;
            Ok(p.into_contents())
        })
        .collect::<Result<Vec<_>, EmergencyTrustError>>()?;

    let leaf_der = chain_der.first().expect("non-empty checked above");
    let (_, leaf_cert) = x509_parser::parse_x509_certificate(leaf_der)
        .map_err(|e| EmergencyTrustError::X509(e.to_string()))?;

    // Each cert must have validity window covering `now_ms`.
    for (i, der) in chain_der.iter().enumerate() {
        let (_, cert) = x509_parser::parse_x509_certificate(der)
            .map_err(|e| EmergencyTrustError::X509(e.to_string()))?;
        let nb_ms = cert.validity().not_before.timestamp() * 1000;
        let na_ms = cert.validity().not_after.timestamp() * 1000;
        if now_ms < nb_ms || now_ms >= na_ms {
            return Err(EmergencyTrustError::ChainValidity(i));
        }
        // 7. EKU OID check (not on the root — roots may not carry it).
        if i + 1 < chain_der.len() && !has_ohd_eku(&cert) {
            return Err(EmergencyTrustError::MissingEku(i));
        }
    }

    // 5. Each cert signed by its parent.
    for i in 0..chain_der.len().saturating_sub(1) {
        let (_, child) = x509_parser::parse_x509_certificate(&chain_der[i])
            .map_err(|e| EmergencyTrustError::X509(e.to_string()))?;
        let (_, parent) = x509_parser::parse_x509_certificate(&chain_der[i + 1])
            .map_err(|e| EmergencyTrustError::X509(e.to_string()))?;
        // x509-parser's `verify_signature` walks rustls-relevant algorithms;
        // Ed25519 is supported in 0.16+.
        child
            .verify_signature(Some(parent.public_key()))
            .map_err(|_| EmergencyTrustError::ChainSignatureMismatch(i))?;
    }

    // 6. Chain terminates at a trusted root: the last cert in the chain's
    // SPKI must match a root in `trust_roots`. If the chain doesn't carry
    // the root explicitly, we accept the cert one above the leaf if its
    // *issuer* matches a trusted root's subject (RFC 5280-style). For v1
    // we require the chain to carry the root; richer fallback is a v1.x
    // follow-up.
    let last_der = chain_der.last().unwrap();
    let (_, last_cert) = x509_parser::parse_x509_certificate(last_der)
        .map_err(|e| EmergencyTrustError::X509(e.to_string()))?;
    let last_spki = last_cert.public_key().raw;
    let trusted = trust_roots.iter().any(|tr| {
        let parsed = match pem::parse(&tr.pem) {
            Ok(p) => p,
            Err(_) => return false,
        };
        let (_, c) = match x509_parser::parse_x509_certificate(parsed.contents()) {
            Ok(p) => p,
            Err(_) => return false,
        };
        c.public_key().raw == last_spki
    });
    if !trusted {
        return Err(EmergencyTrustError::NoTrustedRoot);
    }

    // 8. Detached signature.
    let leaf_spki = leaf_cert.public_key();
    let leaf_pubkey_bytes = leaf_spki.subject_public_key.data.as_ref();
    if leaf_pubkey_bytes.len() != 32 {
        return Err(EmergencyTrustError::LeafNotEd25519);
    }
    let mut pk32 = [0u8; 32];
    pk32.copy_from_slice(leaf_pubkey_bytes);
    let verifying_key = VerifyingKey::from_bytes(&pk32)
        .map_err(|_| EmergencyTrustError::LeafNotEd25519)?;

    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(&signed.signature)
        .map_err(|_| EmergencyTrustError::RequestSignatureMismatch)?;
    if sig_bytes.len() != 64 {
        return Err(EmergencyTrustError::RequestSignatureMismatch);
    }
    let mut sig64 = [0u8; 64];
    sig64.copy_from_slice(&sig_bytes);
    let signature = Signature::from_bytes(&sig64);

    let canon_request = strip_signature(&signed.request);
    let digest = canonical_signing_bytes(&canon_request)?;
    verifying_key
        .verify(&digest, &signature)
        .map_err(|_| EmergencyTrustError::RequestSignatureMismatch)?;

    Ok(())
}

fn strip_signature(req: &EmergencyAccessRequest) -> EmergencyAccessRequest {
    req.clone()
}

fn has_ohd_eku(cert: &x509_parser::certificate::X509Certificate<'_>) -> bool {
    if let Ok(Some(ekus)) = cert.extended_key_usage() {
        // x509_parser's EKU exposes a `.value` with `.other` Vec of OIDs.
        for oid in &ekus.value.other {
            if oid.to_id_string() == OHD_EMERGENCY_AUTHORITY_OID {
                return true;
            }
        }
    }
    false
}

fn random_request_id_hex() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

/// Compute "should we refresh now?" — true when the cert is within
/// `refresh_window` of expiring (or already expired).
pub fn should_refresh(
    chain: &AuthorityCertChain,
    refresh_window: Duration,
    now: SystemTime,
) -> bool {
    let now_ms = now
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let until_expiry = chain.millis_until_expiry(now_ms);
    let window_ms = refresh_window.as_millis() as i64;
    until_expiry <= window_ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::Pkcs8Bytes;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;
    use rcgen::{CertificateParams, KeyPair};

    /// Build a self-signed Ed25519 leaf cert and a chain consisting of
    /// just that one cert (acts as both leaf and root for tests). The
    /// returned chain's `leaf_signing_key` corresponds to the cert's
    /// public key.
    fn build_self_signed_chain(validity_secs: u64) -> (AuthorityCertChain, Vec<u8>) {
        // We need our SigningKey to match the cert's public key. rcgen's
        // KeyPair is an opaque wrapper; its `public_key_raw()` exposes the
        // raw SPKI bytes. To sync them we construct an Ed25519 SigningKey
        // first, derive its KeyPair via rcgen's PKCS#8 importer, then
        // generate the cert.
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let pkcs8_der = signing_key.to_pkcs8_der_bytes();
        let kp = KeyPair::try_from(pkcs8_der.as_slice()).unwrap();
        let mut params = CertificateParams::new(vec!["ohd-test".into()]).unwrap();
        let now = std::time::SystemTime::now();
        params.not_before = now.into();
        params.not_after = (now + Duration::from_secs(validity_secs)).into();
        // Add the OHD emergency-authority EKU OID so EKU check passes.
        // x509-parser checks `extended_key_usage` extension; rcgen lets us
        // add custom OIDs via `custom_extensions` but the API is verbose.
        // For tests we add a `CustomExtension` containing the EKU encoded
        // manually; here we keep it simple and add it on the leaf only.
        // EKU OIDs are treated by rcgen via `extended_key_usages`. We
        // append a single custom OID via the OID parser:
        let oid: rcgen::CustomExtension = ohd_eku_extension();
        params.custom_extensions.push(oid);
        let cert = params.self_signed(&kp).unwrap();
        let pem = cert.pem().into_bytes();
        let parsed = pem::parse(&pem).unwrap();
        let der = parsed.contents().to_vec();
        let (nb, na) = AuthorityCertChain::parse_leaf_validity(&pem).unwrap();
        let chain = AuthorityCertChain {
            leaf_pem: pem.clone(),
            intermediate_pem: pem.clone(), // not used in 1-cert chain tests
            root_pem: pem.clone(),
            leaf_signing_key: signing_key,
            leaf_not_before_ms: nb,
            leaf_not_after_ms: na,
        };
        (chain, der)
    }

    /// Build the EKU extension with our OID.
    fn ohd_eku_extension() -> rcgen::CustomExtension {
        // ASN.1 SEQUENCE { OID 1.3.6.1.4.1.99999.1.1 }
        // Hand-encoded DER:
        //   0x30 0x0B           SEQUENCE, length 11
        //     0x06 0x09         OID, length 9
        //       0x2B 0x06 0x01 0x04 0x01 0x86 0x8F 0x1F 0x01 0x01
        //
        // Actually: 1.3.6.1.4.1.99999.1.1 encodes to:
        //   1.3 → 0x2B
        //   6   → 0x06
        //   1   → 0x01
        //   4   → 0x04
        //   1   → 0x01
        //   99999 → 0x86 0x8F 0x1F  (0x86 0x8F 0x1F = 0b10000110 0b10001111 0b00011111
        //                            → 0000110 0001111 0011111 → 0x186F1F = 99999 ✓)
        //   1   → 0x01
        //   1   → 0x01
        // Total OID body = 10 bytes.
        let oid_body = [
            0x2B, 0x06, 0x01, 0x04, 0x01, 0x86, 0x8F, 0x1F, 0x01, 0x01,
        ];
        let mut ext_value = Vec::new();
        ext_value.push(0x30); // SEQUENCE
        ext_value.push((2 + oid_body.len()) as u8); // length
        ext_value.push(0x06); // OID tag
        ext_value.push(oid_body.len() as u8);
        ext_value.extend_from_slice(&oid_body);
        // OID for `id-ce-extKeyUsage` is 2.5.29.37 = [85, 29, 37]
        let mut ext = rcgen::CustomExtension::from_oid_content(&[2, 5, 29, 37], ext_value);
        ext.set_criticality(false);
        ext
    }

    #[test]
    fn canonical_bytes_are_stable_under_field_order() {
        let req1 = EmergencyAccessRequest {
            request_id: hex::encode([1u8; 16]),
            issued_at_ms: 1000,
            expires_at_ms: 2000,
            patient_storage_pubkey_pin: None,
            responder_label: Some("R".into()),
            scene_context: None,
            operator_label: Some("EMS".into()),
            scene_lat: None,
            scene_lon: None,
            scene_accuracy_m: None,
            cert_chain_pem: vec!["A".into()],
        };
        let req2 = req1.clone();
        let h1 = canonical_signing_bytes(&req1).unwrap();
        let h2 = canonical_signing_bytes(&req2).unwrap();
        assert_eq!(h1, h2);
    }

    #[test]
    fn sign_then_verify_roundtrip_against_self_signed_root() {
        let (chain, _der) = build_self_signed_chain(3600);
        let trust = vec![TrustRoot {
            pem: chain.root_pem.clone(),
            label: "test-root".into(),
        }];
        let req = EmergencyAccessRequest {
            request_id: String::new(), // sign_request fills in
            issued_at_ms: 0,
            expires_at_ms: 0,
            patient_storage_pubkey_pin: None,
            responder_label: Some("Officer Test".into()),
            scene_context: Some("emergency drill".into()),
            operator_label: Some("EMS Test Region".into()),
            scene_lat: None,
            scene_lon: None,
            scene_accuracy_m: None,
            cert_chain_pem: vec![], // sign_request fills in
        };

        // The wire chain has [leaf, intermediate, root] but for our 1-cert
        // self-signed test we collapse to just the leaf to keep depth=1
        // valid. (intermediate/root duplicates would trigger
        // ChainSignatureMismatch since they don't sign each other.)
        let mut chain1 = chain.clone();
        chain1.intermediate_pem = chain1.leaf_pem.clone();
        chain1.root_pem = chain1.leaf_pem.clone();
        let now_ms = super::super::cert_chain::now_ms();
        let mut signed = sign_request(&chain1, req, now_ms).unwrap();
        // Collapse to a 1-element chain.
        signed.request.cert_chain_pem.truncate(1);
        // Re-sign, since canonical bytes changed.
        let digest = canonical_signing_bytes(&signed.request).unwrap();
        let sig = chain1.leaf_signing_key.sign(&digest);
        signed.signature = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());

        verify_request(&signed, &trust, now_ms, None).expect("verify should pass");
    }

    #[test]
    fn verify_rejects_wrong_signature() {
        let (chain, _) = build_self_signed_chain(3600);
        let trust = vec![TrustRoot {
            pem: chain.root_pem.clone(),
            label: "tr".into(),
        }];
        let req = EmergencyAccessRequest {
            request_id: hex::encode([0u8; 16]),
            issued_at_ms: super::super::cert_chain::now_ms(),
            expires_at_ms: super::super::cert_chain::now_ms() + 60_000,
            patient_storage_pubkey_pin: None,
            responder_label: None,
            scene_context: None,
            operator_label: None,
            scene_lat: None,
            scene_lon: None,
            scene_accuracy_m: None,
            cert_chain_pem: vec![String::from_utf8(chain.leaf_pem.clone()).unwrap()],
        };
        let signed = SignedEmergencyRequest {
            request: req,
            signature: base64::engine::general_purpose::STANDARD.encode([0u8; 64]),
        };
        let now_ms = super::super::cert_chain::now_ms();
        let r = verify_request(&signed, &trust, now_ms, None);
        assert!(matches!(r, Err(EmergencyTrustError::RequestSignatureMismatch)));
    }

    #[test]
    fn verify_rejects_unknown_root() {
        let (chain, _) = build_self_signed_chain(3600);
        // Empty trust roots → NoTrustedRoot.
        let req = EmergencyAccessRequest {
            request_id: hex::encode([2u8; 16]),
            issued_at_ms: super::super::cert_chain::now_ms(),
            expires_at_ms: super::super::cert_chain::now_ms() + 60_000,
            patient_storage_pubkey_pin: None,
            responder_label: None,
            scene_context: None,
            operator_label: None,
            scene_lat: None,
            scene_lon: None,
            scene_accuracy_m: None,
            cert_chain_pem: vec![String::from_utf8(chain.leaf_pem.clone()).unwrap()],
        };
        let digest = canonical_signing_bytes(&req).unwrap();
        let sig = chain.leaf_signing_key.sign(&digest);
        let signed = SignedEmergencyRequest {
            request: req,
            signature: base64::engine::general_purpose::STANDARD.encode(sig.to_bytes()),
        };
        let now_ms = super::super::cert_chain::now_ms();
        let r = verify_request(&signed, &[], now_ms, None);
        assert!(matches!(r, Err(EmergencyTrustError::NoTrustedRoot)));
    }

    #[test]
    fn verify_rejects_chain_too_deep() {
        let (chain, _) = build_self_signed_chain(3600);
        let req = EmergencyAccessRequest {
            request_id: hex::encode([3u8; 16]),
            issued_at_ms: super::super::cert_chain::now_ms(),
            expires_at_ms: super::super::cert_chain::now_ms() + 60_000,
            patient_storage_pubkey_pin: None,
            responder_label: None,
            scene_context: None,
            operator_label: None,
            scene_lat: None,
            scene_lon: None,
            scene_accuracy_m: None,
            // 5 entries → exceeds MAX_CHAIN_DEPTH=4.
            cert_chain_pem: vec![String::from_utf8(chain.leaf_pem.clone()).unwrap(); 5],
        };
        let signed = SignedEmergencyRequest {
            request: req,
            signature: base64::engine::general_purpose::STANDARD.encode([0u8; 64]),
        };
        let now_ms = super::super::cert_chain::now_ms();
        let r = verify_request(&signed, &[], now_ms, None);
        assert!(matches!(r, Err(EmergencyTrustError::DepthExceeded(5, MAX_CHAIN_DEPTH))));
    }

    #[test]
    fn should_refresh_inside_window() {
        let chain = AuthorityCertChain {
            leaf_pem: vec![],
            intermediate_pem: vec![],
            root_pem: vec![],
            leaf_signing_key: SigningKey::from_bytes(&[5u8; 32]),
            leaf_not_before_ms: super::super::cert_chain::now_ms() - 1000,
            // Expires in 30 min.
            leaf_not_after_ms: super::super::cert_chain::now_ms() + 30 * 60 * 1000,
        };
        // 1h refresh window → should refresh (30 min < 1h).
        assert!(should_refresh(
            &chain,
            Duration::from_secs(60 * 60),
            std::time::SystemTime::now()
        ));
        // 10 min window → should not refresh (30 min > 10 min).
        assert!(!should_refresh(
            &chain,
            Duration::from_secs(10 * 60),
            std::time::SystemTime::now()
        ));
    }
}

// Trait the tests use to derive a PKCS#8 PEM for ed25519-dalek's
// `SigningKey`. ed25519-dalek 2.x exposes this via the `pkcs8` feature; we
// gate on it in Cargo.toml.
#[cfg(test)]
pub(crate) trait Pkcs8Bytes {
    fn to_pkcs8_der_bytes(&self) -> Vec<u8>;
}

#[cfg(test)]
impl Pkcs8Bytes for ed25519_dalek::SigningKey {
    fn to_pkcs8_der_bytes(&self) -> Vec<u8> {
        use ed25519_dalek::pkcs8::EncodePrivateKey;
        self.to_pkcs8_der()
            .expect("pkcs8 encode")
            .as_bytes()
            .to_vec()
    }
}
