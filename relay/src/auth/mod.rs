//! Authentication primitives for the relay's REST control plane.
//!
//! Currently scoped to **registration-time OIDC gating**: operators can
//! configure the relay with an issuer allowlist, and storage instances must
//! present an `id_token` from one of those issuers to register.
//!
//! See [`oidc`] for the JWKS-backed verifier; it is also used by the
//! `GET /v1/auth/info` discovery endpoint that lets storage know up front
//! which issuers a relay accepts.
//!
//! ## Why a separate module from `auth_mode`
//!
//! `auth_mode/` is the (feature-gated) emergency-authority cert-chain
//! signer — a different concept entirely (Fulcio-issued ed25519 leaves
//! signing emergency-access requests). Mixing the two would muddle two
//! independent trust domains, and JWT verification needs to work in
//! permissive (non-`authority`) builds too.

pub mod oidc;

pub use oidc::{
    OidcVerifier, OidcVerifierConfig, OidcVerifyError, VerifiedIdToken,
};
