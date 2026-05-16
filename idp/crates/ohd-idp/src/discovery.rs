//! OIDC discovery document (`/.well-known/openid-configuration`).
//!
//! Every endpoint URL is derived from the configured `issuer`, so the
//! same code produces a correct document for `accounts.ohd.dev` and for
//! a clinic's own domain. The endpoints themselves (`/authorize`,
//! `/token`, `/userinfo`) are later phases — discovery just advertises
//! them so an RP's library can be pointed at the issuer today.

use serde::Serialize;

/// The OIDC discovery document (a subset of OpenID Connect Discovery 1.0,
/// covering what an OHD relying party needs).
#[derive(Debug, Clone, Serialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    pub userinfo_endpoint: String,
    pub response_types_supported: Vec<String>,
    pub subject_types_supported: Vec<String>,
    pub id_token_signing_alg_values_supported: Vec<String>,
    pub scopes_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
}

impl Discovery {
    /// Build the discovery document for an `issuer` (no trailing slash).
    pub fn for_issuer(issuer: &str) -> Self {
        let issuer = issuer.trim_end_matches('/');
        Self {
            issuer: issuer.to_string(),
            authorization_endpoint: format!("{issuer}/authorize"),
            token_endpoint: format!("{issuer}/token"),
            jwks_uri: format!("{issuer}/jwks"),
            userinfo_endpoint: format!("{issuer}/userinfo"),
            response_types_supported: vec!["code".to_string()],
            subject_types_supported: vec!["public".to_string()],
            id_token_signing_alg_values_supported: vec!["RS256".to_string()],
            scopes_supported: vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
            ],
            code_challenge_methods_supported: vec!["S256".to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_endpoints_derive_from_issuer() {
        let d = Discovery::for_issuer("https://accounts.ohd.dev");
        assert_eq!(d.issuer, "https://accounts.ohd.dev");
        assert_eq!(d.authorization_endpoint, "https://accounts.ohd.dev/authorize");
        assert_eq!(d.token_endpoint, "https://accounts.ohd.dev/token");
        assert_eq!(d.jwks_uri, "https://accounts.ohd.dev/jwks");
        assert_eq!(d.userinfo_endpoint, "https://accounts.ohd.dev/userinfo");
        assert_eq!(d.id_token_signing_alg_values_supported, ["RS256"]);
        assert_eq!(d.code_challenge_methods_supported, ["S256"]);
        assert_eq!(d.response_types_supported, ["code"]);
        assert_eq!(d.subject_types_supported, ["public"]);
        assert!(d.scopes_supported.contains(&"openid".to_string()));
    }

    #[test]
    fn trailing_slash_in_issuer_is_trimmed() {
        let d = Discovery::for_issuer("https://accounts.ohd.dev/");
        assert_eq!(d.issuer, "https://accounts.ohd.dev");
        assert_eq!(d.jwks_uri, "https://accounts.ohd.dev/jwks");
    }
}
