//! Authority cert chain: PEM bytes + parsed leaf metadata + Ed25519 keypair.
//!
//! Per `spec/emergency-trust.md`:
//!
//! ```text
//! OHD Project Root CA (10y, offline)
//!   └── OHD Global Fulcio (1y intermediate)
//!         └── Org cert (24h, daily-refreshed)
//!               └── Responder cert (1-4h, optional)
//! ```
//!
//! This struct holds the relay-side perspective: leaf PEM + leaf keypair
//! + chain (intermediate, root). The `signer` module consumes these to
//! produce + verify `EmergencyAccessRequest` signatures.

use std::time::SystemTime;

use ed25519_dalek::SigningKey;

use super::AuthorityError;

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("PEM decode: {0}")]
    Pem(String),
    #[error("X.509 parse: {0}")]
    X509(String),
    #[error("leaf is missing or invalid: {0}")]
    BadLeaf(String),
    #[error("leaf cert is expired or not yet valid")]
    Expired,
    #[error("chain signature mismatch")]
    BadSignature,
    #[error("chain depth {0} exceeds maximum 4 (per spec)")]
    DepthExceeded(usize),
    #[error("chain does not terminate at any trusted root")]
    NoTrustedRoot,
    #[error("missing required OHD emergency-authority EKU OID")]
    MissingEku,
}

/// The relay's currently active authority cert chain.
///
/// Refreshed daily; replaced atomically when a new chain is fetched from
/// Fulcio. The leaf keypair is held in process memory in v1; an HSM-backed
/// signer is a v1.x follow-up.
#[derive(Clone)]
pub struct AuthorityCertChain {
    /// Leaf cert (the org's daily-refresh cert). PEM-encoded.
    pub leaf_pem: Vec<u8>,
    /// Fulcio intermediate cert. PEM-encoded.
    pub intermediate_pem: Vec<u8>,
    /// OHD project root cert. PEM-encoded. May be omitted on the wire if
    /// the patient phone already trusts it.
    pub root_pem: Vec<u8>,
    /// Leaf cert's Ed25519 keypair.
    pub leaf_signing_key: SigningKey,
    /// `notAfter` of the leaf cert (UNIX-ms).
    pub leaf_not_after_ms: i64,
    /// `notBefore` of the leaf cert (UNIX-ms).
    pub leaf_not_before_ms: i64,
}

impl std::fmt::Debug for AuthorityCertChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthorityCertChain")
            .field("leaf_not_before_ms", &self.leaf_not_before_ms)
            .field("leaf_not_after_ms", &self.leaf_not_after_ms)
            .finish()
    }
}

impl AuthorityCertChain {
    /// Returns `true` if the leaf cert is within its validity window at
    /// `now_ms`.
    pub fn is_current(&self, now_ms: i64) -> bool {
        now_ms >= self.leaf_not_before_ms && now_ms < self.leaf_not_after_ms
    }

    /// Milliseconds until the leaf cert expires, clamped to zero if
    /// already expired.
    pub fn millis_until_expiry(&self, now_ms: i64) -> i64 {
        (self.leaf_not_after_ms - now_ms).max(0)
    }

    /// PEM-encode the wire form of the chain (leaf first, root last).
    /// This is what a responder relay would put in
    /// `EmergencyAccessRequest.cert_chain_pem`.
    pub fn wire_chain_pem(&self) -> Vec<Vec<u8>> {
        vec![
            self.leaf_pem.clone(),
            self.intermediate_pem.clone(),
            self.root_pem.clone(),
        ]
    }

    /// Parse the leaf PEM and pull validity timestamps. Used during
    /// construction.
    pub fn parse_leaf_validity(leaf_pem: &[u8]) -> Result<(i64, i64), ChainError> {
        let parsed = pem::parse(leaf_pem).map_err(|e| ChainError::Pem(e.to_string()))?;
        let (_, cert) = x509_parser::parse_x509_certificate(parsed.contents())
            .map_err(|e| ChainError::X509(e.to_string()))?;
        let nb = cert
            .validity()
            .not_before
            .timestamp()
            * 1000;
        let na = cert
            .validity()
            .not_after
            .timestamp()
            * 1000;
        Ok((nb, na))
    }

    /// Sanity-check this chain against `now`. Returns `Err` on expiry, but
    /// does NOT verify chain signatures — that's `signer::verify_chain`.
    /// This is the cheap "is the cert still wall-clock-valid" check used
    /// before signing an outgoing request.
    pub fn check_validity(&self, now_ms: i64) -> Result<(), AuthorityError> {
        if !self.is_current(now_ms) {
            return Err(AuthorityError::Chain(ChainError::Expired));
        }
        Ok(())
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rcgen::{CertificateParams, KeyPair};

    /// Build a self-signed Ed25519 cert via `rcgen` for unit tests.
    /// This isn't a real Fulcio chain — it's a one-cert chain we use to
    /// exercise the validity / expiry code without a network round-trip.
    fn self_signed_pem(validity_secs: u64) -> Vec<u8> {
        let kp = KeyPair::generate_for(&rcgen::PKCS_ED25519).unwrap();
        let mut params = CertificateParams::new(vec!["ohd-test".into()]).unwrap();
        let now = std::time::SystemTime::now();
        params.not_before = now.into();
        params.not_after = (now + std::time::Duration::from_secs(validity_secs)).into();
        let cert = params.self_signed(&kp).unwrap();
        cert.pem().into_bytes()
    }

    #[test]
    fn parse_validity_extracts_timestamps() {
        let pem = self_signed_pem(3600);
        let (nb, na) = AuthorityCertChain::parse_leaf_validity(&pem).unwrap();
        assert!(na > nb);
        assert!(na - nb >= 3000_000); // close to 3600s = 3.6M ms (rcgen rounds to whole seconds)
    }

    #[test]
    fn is_current_checks_window() {
        let chain = AuthorityCertChain {
            leaf_pem: vec![],
            intermediate_pem: vec![],
            root_pem: vec![],
            leaf_signing_key: SigningKey::from_bytes(&[7u8; 32]),
            leaf_not_before_ms: 1000,
            leaf_not_after_ms: 2000,
        };
        assert!(!chain.is_current(500));
        assert!(chain.is_current(1500));
        assert!(!chain.is_current(2500));
        assert_eq!(chain.millis_until_expiry(1500), 500);
        assert_eq!(chain.millis_until_expiry(3000), 0);
    }

    #[test]
    fn wire_chain_orders_leaf_first_root_last() {
        let chain = AuthorityCertChain {
            leaf_pem: b"LEAF".to_vec(),
            intermediate_pem: b"INTER".to_vec(),
            root_pem: b"ROOT".to_vec(),
            leaf_signing_key: SigningKey::from_bytes(&[1u8; 32]),
            leaf_not_before_ms: 0,
            leaf_not_after_ms: 1,
        };
        let chain_pem = chain.wire_chain_pem();
        assert_eq!(chain_pem.len(), 3);
        assert_eq!(chain_pem[0], b"LEAF");
        assert_eq!(chain_pem[1], b"INTER");
        assert_eq!(chain_pem[2], b"ROOT");
    }
}
