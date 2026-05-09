//! Emergency-authority mode (feature-gated behind `authority`).
//!
//! When enabled, the relay additionally:
//!
//! - Holds a Fulcio-issued 24h authority cert chain
//!   `[org_cert, fulcio_intermediate, ohd_root]`.
//! - Runs a daily refresh against an OHD-operated Fulcio's standard
//!   `POST /api/v2/signingCert` endpoint.
//! - Signs `EmergencyAccessRequest` Protobuf payloads with the leaf cert's
//!   Ed25519 private key (over the canonical encoding with the `signature`
//!   field zeroed).
//! - Optionally logs each issued cert to a Rekor transparency log.
//!
//! See `../spec/emergency-trust.md` for the full trust model.
//!
//! ## What's wired (this pass)
//!
//! - **Cert chain types**: [`AuthorityCertChain`] holds the PEM chain
//!   plus the leaf's parsed metadata + the Ed25519 keypair.
//! - **Signed requests**: [`signer::sign_request`] /
//!   [`signer::verify_request`] implement standard X.509 chain validation
//!   + Ed25519 detached signature over the canonical Protobuf encoding
//!   with `signature` field zeroed (per `spec/emergency-trust.md`).
//! - **Fulcio client**: [`fulcio::FulcioClient`] talks the standard v2
//!   `signingCert` endpoint with the OIDC + proof-of-possession body shape
//!   from the spec.
//! - **Rekor client**: [`rekor::RekorClient`] submits a minimal v1 log
//!   entry per refresh; soft-failures are logged but don't block cert use.
//! - **Refresh scheduler**: [`refresh::AuthorityState`] caches the active
//!   chain, exposes a `current()` accessor, and refreshes ~1h before
//!   expiry (driven by a tokio task `refresh::run_refresh_loop`).
//!
//! ## What's still hand-wave-y
//!
//! - **Patient-side full RFC 5280 path validation**: we verify chain
//!   signatures + validity windows + the OHD EKU OID, and pin the chain to
//!   one of the trust roots passed in. We do NOT yet enforce
//!   `pathLenConstraint`, name constraints, or the full RFC 5280 algorithm
//!   — adequate for v1 where the chain shape is fixed (root → fulcio →
//!   org → optional responder), but a follow-up for v1.x.
//! - **Rekor inclusion-proof verification on the verifier side**: optional
//!   per spec, deferred to v1.x.
//!
//! Without `--features authority` the whole module is empty: plain
//! forwarding relays don't pull in the Fulcio / X.509 / Ed25519 dep stack.

#![cfg(feature = "authority")]

pub mod cert_chain;
pub mod fulcio;
pub mod rekor;
pub mod refresh;
pub mod signer;

pub use cert_chain::{AuthorityCertChain, ChainError};
pub use fulcio::{FulcioClient, FulcioConfig, FulcioRequest, FulcioResponse};
pub use rekor::{RekorClient, RekorConfig, RekorEntry};
pub use refresh::{AuthorityState, AuthorityStateConfig};
pub use signer::{
    canonical_signing_bytes, sign_request, verify_request, EmergencyAccessRequest,
    EmergencyTrustError, SignedEmergencyRequest, TrustRoot,
};

#[derive(Debug, thiserror::Error)]
pub enum AuthorityError {
    #[error("authority cert chain: {0}")]
    Chain(#[from] ChainError),
    #[error("fulcio refresh: {0}")]
    Fulcio(String),
    #[error("rekor: {0}")]
    Rekor(String),
    #[error("signing: {0}")]
    Signing(#[from] EmergencyTrustError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("config: {0}")]
    Config(String),
}
