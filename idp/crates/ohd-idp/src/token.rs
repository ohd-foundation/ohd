//! `id_token` minting — RS256 JWTs signed with the IdP's signing key.
//!
//! The `id_token` an OHD relying party receives carries the OHD identity:
//! `sub` is the `profile_ulid`, the same stable ULID OHD SaaS mints. Every
//! RP that trusts `accounts.ohd.dev` therefore identifies a user the same
//! way (see `SPEC.md` — "The `id_token`").

use crate::keys::SigningKey;
use anyhow::{Context, Result};
use jsonwebtoken::{Algorithm, Header};
use serde::{Deserialize, Serialize};

/// The `id_token` claim set. A subset of OpenID Connect Core's `IDToken`,
/// covering what an OHD RP needs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdTokenClaims {
    /// Issuer — the configured `iss`, exact.
    pub iss: String,
    /// Subject — the `profile_ulid`, the stable OHD identity.
    pub sub: String,
    /// Audience — the RP's `client_id`.
    pub aud: String,
    /// Expiry (Unix seconds).
    pub exp: i64,
    /// Issued-at (Unix seconds).
    pub iat: i64,
    /// When the end-user authenticated (Unix seconds).
    pub auth_time: i64,
    /// The RP-supplied `nonce`, echoed back. Omitted if the RP sent none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    /// The user's email.
    pub email: String,
    /// Whether the IdP considers the email verified. The first-party
    /// email/password path does not run an email-confirmation step, so
    /// this is `false` — honest about what the IdP has actually checked.
    pub email_verified: bool,
}

/// How long a minted `id_token` is valid.
const ID_TOKEN_TTL_SECS: i64 = 3600;

/// Mint a signed RS256 `id_token`.
///
/// `auth_time` is when the user actually authenticated this request;
/// `iat`/`exp` are derived from now. The JWT header carries the signing
/// key's stable `kid` so an RP can pick the right JWKS entry.
#[allow(clippy::too_many_arguments)]
pub fn mint_id_token(
    key: &SigningKey,
    issuer: &str,
    profile_ulid: &str,
    client_id: &str,
    email: &str,
    email_verified: bool,
    nonce: Option<&str>,
    auth_time: i64,
) -> Result<String> {
    let now = now_unix();
    let claims = IdTokenClaims {
        iss: issuer.to_string(),
        sub: profile_ulid.to_string(),
        aud: client_id.to_string(),
        exp: now + ID_TOKEN_TTL_SECS,
        iat: now,
        auth_time,
        nonce: nonce.map(str::to_string),
        email: email.to_string(),
        email_verified,
    };
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(key.kid().to_string());
    let encoding = key.encoding_key().context("building RS256 encoding key")?;
    jsonwebtoken::encode(&header, &claims, &encoding).context("signing id_token")
}

/// The lifetime, in seconds, of issued access tokens — surfaced as
/// `expires_in` in the token response.
pub const ACCESS_TOKEN_TTL_SECS: i64 = 3600;

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{decode, decode_header, DecodingKey, Validation};

    #[test]
    fn id_token_carries_the_expected_claims_and_verifies() {
        let key = SigningKey::generate().unwrap();
        let jwt = mint_id_token(
            &key,
            "https://accounts.ohd.dev",
            "01PROFILEULID00000000000000",
            "cord-web",
            "user@example.com",
            false,
            Some("nonce-123"),
            1_700_000_000,
        )
        .unwrap();

        // Header carries the kid.
        let header = decode_header(&jwt).unwrap();
        assert_eq!(header.alg, Algorithm::RS256);
        assert_eq!(header.kid.as_deref(), Some(key.kid()));

        // Verify with the public JWK material.
        let jwk = crate::jwks::Jwk::from_signing_key(&key);
        let decoding =
            DecodingKey::from_rsa_components(&jwk.n, &jwk.e).expect("rsa components decode");
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&["https://accounts.ohd.dev"]);
        validation.set_audience(&["cord-web"]);
        let data = decode::<IdTokenClaims>(&jwt, &decoding, &validation).unwrap();
        let c = data.claims;
        assert_eq!(c.iss, "https://accounts.ohd.dev");
        assert_eq!(c.sub, "01PROFILEULID00000000000000");
        assert_eq!(c.aud, "cord-web");
        assert_eq!(c.nonce.as_deref(), Some("nonce-123"));
        assert_eq!(c.email, "user@example.com");
        assert!(!c.email_verified);
        assert_eq!(c.auth_time, 1_700_000_000);
        assert!(c.exp > c.iat);
    }

    #[test]
    fn id_token_without_nonce_omits_the_claim() {
        let key = SigningKey::generate().unwrap();
        let jwt = mint_id_token(
            &key,
            "https://accounts.ohd.dev",
            "01SUB",
            "connect-web",
            "u@e.com",
            false,
            None,
            1_700_000_000,
        )
        .unwrap();
        // Decode the payload directly to confirm `nonce` is absent.
        let payload = jwt.split('.').nth(1).unwrap();
        use base64::engine::general_purpose::URL_SAFE_NO_PAD;
        use base64::Engine;
        let bytes = URL_SAFE_NO_PAD.decode(payload).unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(v.get("nonce").is_none());
    }
}
