//! Standard Sigstore Fulcio v2 client.
//!
//! Per `spec/emergency-trust.md`:
//!
//! ```text
//! POST /api/v2/signingCert
//! Authorization: Bearer <OIDC token from OHD's emergency-authority OIDC provider>
//! Content-Type: application/json
//!
//! {
//!   "credentials": {
//!     "oidcIdentityToken": "<JWT from OIDC provider>"
//!   },
//!   "publicKeyRequest": {
//!     "publicKey": {
//!       "algorithm": "ED25519",
//!       "content": "<base64 SPKI of org's daily-refresh keypair>"
//!     },
//!     "proofOfPossession": "<Ed25519 signature over OIDC token's email claim>"
//!   }
//! }
//!
//! → 201 Created
//! {
//!   "signedCertificateEmbeddedSct": {
//!     "chain": {
//!       "certificates": ["<PEM of org cert>", "<PEM of intermediate>"]
//!     }
//!   }
//! }
//! ```
//!
//! We hand-roll the HTTP because the `sigstore` crate is heavy +
//! experimental and we only need this one endpoint. The wire shape is
//! pinned by Sigstore's OpenAPI spec; if Fulcio v3 ships, this module
//! grows alongside.
//!
//! Proof-of-possession is an Ed25519 signature over the SHA-256 of the
//! OIDC token's `email` claim — Fulcio binds the issued cert to that
//! email so it appears in the cert's SAN.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use reqwest::Client as HttpClient;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::cert_chain::AuthorityCertChain;
use super::AuthorityError;

/// Minimal config for the Fulcio client. Hostnames typically come from
/// `relay.toml`'s `[authority]` section.
#[derive(Debug, Clone)]
pub struct FulcioConfig {
    /// Fulcio base URL, e.g. `"https://fulcio.openhealth-data.org"`.
    pub fulcio_url: String,
    /// Override for testing — point at a local stub.
    pub override_signing_cert_url: Option<String>,
}

impl FulcioConfig {
    fn signing_cert_url(&self) -> String {
        if let Some(o) = &self.override_signing_cert_url {
            return o.clone();
        }
        format!("{}/api/v2/signingCert", self.fulcio_url.trim_end_matches('/'))
    }
}

/// HTTP client for Fulcio. Cheap to clone.
#[derive(Clone)]
pub struct FulcioClient {
    cfg: Arc<FulcioConfig>,
    http: HttpClient,
}

impl std::fmt::Debug for FulcioClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FulcioClient")
            .field("fulcio_url", &self.cfg.fulcio_url)
            .finish()
    }
}

impl FulcioClient {
    pub fn new(cfg: FulcioConfig) -> Result<Self, AuthorityError> {
        let http = HttpClient::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| AuthorityError::Fulcio(format!("build http: {e}")))?;
        Ok(Self {
            cfg: Arc::new(cfg),
            http,
        })
    }

    pub fn with_http(cfg: FulcioConfig, http: HttpClient) -> Self {
        Self {
            cfg: Arc::new(cfg),
            http,
        }
    }

    /// Exchange an OIDC ID-token for a fresh authority cert chain.
    ///
    /// `oidc_id_token` — the bearer the OHD emergency-authority OIDC IdP
    /// issued for the org's relay account.
    /// `email_claim` — the `email` claim from the OIDC ID-token; Fulcio
    /// asks the caller to prove possession of the keypair by signing this.
    /// We could parse the JWT here and pull the claim ourselves, but
    /// keeping this API explicit is simpler — the caller (refresh loop)
    /// already has the parsed token.
    /// `signing_key` — the Ed25519 keypair the cert will bind to.
    pub async fn refresh_cert(
        &self,
        oidc_id_token: &str,
        email_claim: &str,
        signing_key: &SigningKey,
    ) -> Result<AuthorityCertChain, AuthorityError> {
        let pubkey_bytes = signing_key.verifying_key().to_bytes();
        // Fulcio expects the SPKI-encoded public key, but for Ed25519 the
        // raw 32-byte pubkey is also accepted by Sigstore's reference
        // implementation. We send the raw bytes wrapped in base64 — the
        // simplest interop shape.
        let pubkey_b64 = base64::engine::general_purpose::STANDARD.encode(pubkey_bytes);

        // Proof-of-possession: Ed25519 sig over SHA-256(email_claim).
        let mut h = Sha256::new();
        h.update(email_claim.as_bytes());
        let digest = h.finalize();
        let sig = signing_key.sign(&digest);
        let pop_b64 = base64::engine::general_purpose::STANDARD.encode(sig.to_bytes());

        let body = FulcioRequest {
            credentials: FulcioCredentials {
                oidc_identity_token: oidc_id_token.to_string(),
            },
            public_key_request: FulcioPublicKeyRequest {
                public_key: FulcioPublicKey {
                    algorithm: "ED25519".to_string(),
                    content: pubkey_b64,
                },
                proof_of_possession: pop_b64,
            },
        };

        let url = self.cfg.signing_cert_url();
        let resp = self
            .http
            .post(&url)
            .bearer_auth(oidc_id_token)
            .json(&body)
            .send()
            .await
            .map_err(|e| AuthorityError::Fulcio(format!("post: {e}")))?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            let text = resp.text().await.unwrap_or_default();
            return Err(AuthorityError::Fulcio(format!("{status}: {text}")));
        }
        let parsed: FulcioResponse = resp
            .json()
            .await
            .map_err(|e| AuthorityError::Fulcio(format!("parse: {e}")))?;

        // Pluck out the chain. Sigstore returns 2 PEMs typically (org cert
        // + intermediate); the OHD-trusted root is implicit (clients hold
        // it locally). For wire portability we still send the root in our
        // SignedEmergencyRequest, but on the refresh path we may or may
        // not have it from Fulcio.
        let certs = parsed
            .signed_certificate_embedded_sct
            .chain
            .certificates;
        if certs.is_empty() {
            return Err(AuthorityError::Fulcio("empty chain returned".into()));
        }
        let leaf_pem = certs[0].as_bytes().to_vec();
        let intermediate_pem = certs.get(1).map(|s| s.as_bytes().to_vec()).unwrap_or_default();
        // Some Fulcio deployments include the root; some don't.
        let root_pem = certs.get(2).map(|s| s.as_bytes().to_vec()).unwrap_or_default();
        let (nb, na) = AuthorityCertChain::parse_leaf_validity(&leaf_pem)?;

        Ok(AuthorityCertChain {
            leaf_pem,
            intermediate_pem,
            root_pem,
            leaf_signing_key: signing_key.clone(),
            leaf_not_before_ms: nb,
            leaf_not_after_ms: na,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct FulcioRequest {
    pub credentials: FulcioCredentials,
    #[serde(rename = "publicKeyRequest")]
    pub public_key_request: FulcioPublicKeyRequest,
}

#[derive(Debug, Serialize)]
pub struct FulcioCredentials {
    #[serde(rename = "oidcIdentityToken")]
    pub oidc_identity_token: String,
}

#[derive(Debug, Serialize)]
pub struct FulcioPublicKeyRequest {
    #[serde(rename = "publicKey")]
    pub public_key: FulcioPublicKey,
    #[serde(rename = "proofOfPossession")]
    pub proof_of_possession: String,
}

#[derive(Debug, Serialize)]
pub struct FulcioPublicKey {
    pub algorithm: String,
    pub content: String,
}

#[derive(Debug, Deserialize)]
pub struct FulcioResponse {
    #[serde(rename = "signedCertificateEmbeddedSct")]
    pub signed_certificate_embedded_sct: FulcioChainContainer,
}

#[derive(Debug, Deserialize)]
pub struct FulcioChainContainer {
    pub chain: FulcioChain,
}

#[derive(Debug, Deserialize)]
pub struct FulcioChain {
    pub certificates: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signing_cert_url_uses_override() {
        let cfg = FulcioConfig {
            fulcio_url: "https://example".into(),
            override_signing_cert_url: Some("http://127.0.0.1:8080/sign".into()),
        };
        assert_eq!(cfg.signing_cert_url(), "http://127.0.0.1:8080/sign");
    }

    #[test]
    fn signing_cert_url_appends_path() {
        let cfg = FulcioConfig {
            fulcio_url: "https://fulcio.example/".into(),
            override_signing_cert_url: None,
        };
        assert_eq!(
            cfg.signing_cert_url(),
            "https://fulcio.example/api/v2/signingCert"
        );
    }

    #[test]
    fn fulcio_request_serializes_to_expected_shape() {
        let req = FulcioRequest {
            credentials: FulcioCredentials {
                oidc_identity_token: "tok".into(),
            },
            public_key_request: FulcioPublicKeyRequest {
                public_key: FulcioPublicKey {
                    algorithm: "ED25519".into(),
                    content: "AAAA".into(),
                },
                proof_of_possession: "BBBB".into(),
            },
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["credentials"]["oidcIdentityToken"], "tok");
        assert_eq!(v["publicKeyRequest"]["publicKey"]["algorithm"], "ED25519");
        assert_eq!(v["publicKeyRequest"]["publicKey"]["content"], "AAAA");
        assert_eq!(v["publicKeyRequest"]["proofOfPossession"], "BBBB");
    }

    #[test]
    fn fulcio_response_parses_chain() {
        let raw = r#"{
            "signedCertificateEmbeddedSct": {
                "chain": {
                    "certificates": ["LEAF_PEM", "INTER_PEM"]
                }
            }
        }"#;
        let parsed: FulcioResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(
            parsed.signed_certificate_embedded_sct.chain.certificates,
            vec!["LEAF_PEM".to_string(), "INTER_PEM".to_string()]
        );
    }
}
