//! JSON Web Key Set construction.
//!
//! Builds the `/jwks` document from the IdP's signing keys. The set is a
//! `Vec` of [`Jwk`] so a rotation-overlap key can simply be appended —
//! RPs pick the right key by matching the `id_token` header `kid`.

use crate::keys::SigningKey;
use serde::Serialize;

/// One RSA public key in JWK form (RFC 7517).
#[derive(Debug, Clone, Serialize)]
pub struct Jwk {
    /// Key type — always `RSA` here.
    pub kty: String,
    /// Intended use — `sig` (signature verification).
    #[serde(rename = "use")]
    pub use_: String,
    /// Algorithm the key signs with.
    pub alg: String,
    /// Key id — matches the `id_token` header `kid`.
    pub kid: String,
    /// Base64url RSA modulus.
    pub n: String,
    /// Base64url RSA public exponent.
    pub e: String,
}

impl Jwk {
    /// Build the JWK for an RS256 signing key.
    pub fn from_signing_key(key: &SigningKey) -> Self {
        Self {
            kty: "RSA".to_string(),
            use_: "sig".to_string(),
            alg: "RS256".to_string(),
            kid: key.kid().to_string(),
            n: key.jwk_modulus(),
            e: key.jwk_exponent(),
        }
    }
}

/// The full JWKS document published at `/jwks`.
#[derive(Debug, Clone, Serialize)]
pub struct Jwks {
    pub keys: Vec<Jwk>,
}

impl Jwks {
    /// Build a JWKS containing just the current signing key. Rotation
    /// (a later phase) appends the overlapping prior public key here.
    pub fn from_current(key: &SigningKey) -> Self {
        Self {
            keys: vec![Jwk::from_signing_key(key)],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jwks_shape_matches_rfc7517() {
        let key = SigningKey::generate().unwrap();
        let jwks = Jwks::from_current(&key);
        assert_eq!(jwks.keys.len(), 1);
        let jwk = &jwks.keys[0];
        assert_eq!(jwk.kty, "RSA");
        assert_eq!(jwk.use_, "sig");
        assert_eq!(jwk.alg, "RS256");
        assert_eq!(jwk.kid, key.kid());
        assert!(!jwk.n.is_empty());
        assert!(!jwk.e.is_empty());

        // Serialized form uses the `use` wire name, not `use_`.
        let v = serde_json::to_value(&jwks).unwrap();
        assert!(v["keys"][0]["use"].is_string());
        assert!(v["keys"][0].get("use_").is_none());
    }
}
