//! OIDC **Relying Party** provider catalog for the storage server's AS.
//!
//! The storage AS plays two OAuth roles (see `connect/spec/auth.md` "Role
//! split"): an Authorization Server toward OHD clients, and an OIDC Relying
//! Party toward identity providers. This module owns the RP side — the
//! catalog of upstream OIDC providers a remote/cloud storage deployment will
//! delegate "who are you?" to.
//!
//! # Config-driven, deployment-mode aware
//!
//! Providers are configured at launch (`--oauth-provider` on the `serve`
//! subcommand). On-device / self-hosted storage configures *no* provider and
//! keeps the paste-a-self-session-token login UX; server/cloud storage
//! configures at least `ohd_account` so a remote user can "Sign in with OHD".
//!
//! # The `ohd_account` provider
//!
//! `ohd_account` is the OHD project's first-party OIDC provider — the
//! `ohd-idp` service deployed at `accounts.ohd.dev`. It is "just another OIDC
//! provider" from the protocol's view: discovery at
//! `https://accounts.ohd.dev/.well-known/openid-configuration`, RS256
//! id_tokens whose `sub` is the user's stable OHD `profile_ulid`. Selecting it
//! needs no provider-side secret because the `connect-web` client registered
//! at the IdP is a public (PKCE-only) client — but the storage AS *itself*
//! acts as the RP here, so it uses its own registered client. For the v1
//! slice the storage RP authenticates to `ohd-idp` as a public client too
//! (PKCE), which `ohd-idp`'s registry supports (`public = true`).

/// A configured upstream OIDC provider the storage AS can delegate login to.
#[derive(Debug, Clone)]
pub struct OidcProvider {
    /// Stable catalog key (`ohd_account`, `google`, … or a custom slug).
    pub key: String,
    /// Human-facing label shown on the storage login page.
    pub display_name: String,
    /// The provider's OIDC issuer URL — also its `iss` claim and the base for
    /// `<issuer>/.well-known/openid-configuration` discovery.
    pub issuer: String,
    /// OAuth `client_id` the storage AS uses as an RP of this provider.
    pub client_id: String,
    /// OAuth `client_secret`. `None` => public client (PKCE only).
    pub client_secret: Option<String>,
    /// OAuth scopes requested. Always includes `openid`.
    pub scopes: String,
}

impl OidcProvider {
    /// The built-in `ohd_account` provider — OHD Identity at `accounts.ohd.dev`.
    ///
    /// `client_id` defaults to `ohd-storage` (matching what an `ohd-idp`
    /// operator registers for storage RPs); override via config when an
    /// operator runs their own `accounts.<domain>`.
    pub fn ohd_account() -> Self {
        Self {
            key: "ohd_account".to_string(),
            display_name: "OHD Account".to_string(),
            issuer: "https://accounts.ohd.dev".to_string(),
            client_id: "ohd-storage".to_string(),
            client_secret: None,
            scopes: "openid email profile".to_string(),
        }
    }

    /// Parse a `--oauth-provider` CLI value into a provider.
    ///
    /// Forms accepted:
    ///   - `ohd_account` — the built-in, no further config needed.
    ///   - `ohd_account=https://accounts.example.org` — built-in shape, custom issuer.
    ///   - `KEY=ISSUER` — a custom provider; `client_id` defaults to `ohd-storage`.
    ///
    /// `client_id` / `client_secret` for non-default cases are supplied via
    /// the `OHD_OAUTH_PROVIDER_<KEY>_CLIENT_ID` /
    /// `OHD_OAUTH_PROVIDER_<KEY>_CLIENT_SECRET` env vars (read at
    /// [`apply_env_overrides`]).
    pub fn parse_spec(spec: &str) -> Result<Self, String> {
        let (key, issuer) = match spec.split_once('=') {
            Some((k, v)) => (k.trim(), Some(v.trim().to_string())),
            None => (spec.trim(), None),
        };
        if key.is_empty() {
            return Err("empty provider key".into());
        }
        let mut provider = if key == "ohd_account" {
            Self::ohd_account()
        } else {
            Self {
                key: key.to_string(),
                display_name: key.to_string(),
                issuer: String::new(),
                client_id: "ohd-storage".to_string(),
                client_secret: None,
                scopes: "openid email profile".to_string(),
            }
        };
        if let Some(iss) = issuer {
            provider.issuer = iss;
        }
        if provider.issuer.is_empty() {
            return Err(format!(
                "provider {key:?} has no issuer URL — pass it as KEY=ISSUER"
            ));
        }
        provider.apply_env_overrides();
        Ok(provider)
    }

    /// Layer `OHD_OAUTH_PROVIDER_<KEY>_*` env vars over the parsed defaults.
    /// Keeps secrets out of the process argv.
    fn apply_env_overrides(&mut self) {
        let upper = self.key.to_uppercase();
        if let Ok(v) = std::env::var(format!("OHD_OAUTH_PROVIDER_{upper}_CLIENT_ID")) {
            if !v.is_empty() {
                self.client_id = v;
            }
        }
        if let Ok(v) = std::env::var(format!("OHD_OAUTH_PROVIDER_{upper}_CLIENT_SECRET")) {
            if !v.is_empty() {
                self.client_secret = Some(v);
            }
        }
        if let Ok(v) = std::env::var(format!("OHD_OAUTH_PROVIDER_{upper}_ISSUER")) {
            if !v.is_empty() {
                self.issuer = v;
            }
        }
    }

    /// Discovery URL for this provider.
    pub fn discovery_url(&self) -> String {
        format!(
            "{}/.well-known/openid-configuration",
            self.issuer.trim_end_matches('/')
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ohd_account_builtin_parses() {
        let p = OidcProvider::parse_spec("ohd_account").unwrap();
        assert_eq!(p.key, "ohd_account");
        assert_eq!(p.issuer, "https://accounts.ohd.dev");
        assert_eq!(p.display_name, "OHD Account");
        assert_eq!(
            p.discovery_url(),
            "https://accounts.ohd.dev/.well-known/openid-configuration"
        );
    }

    #[test]
    fn ohd_account_custom_issuer() {
        let p = OidcProvider::parse_spec("ohd_account=https://accounts.clinic.org").unwrap();
        assert_eq!(p.key, "ohd_account");
        assert_eq!(p.issuer, "https://accounts.clinic.org");
    }

    #[test]
    fn custom_provider_requires_issuer() {
        assert!(OidcProvider::parse_spec("google").is_err());
        let p = OidcProvider::parse_spec("google=https://accounts.google.com").unwrap();
        assert_eq!(p.key, "google");
        assert_eq!(p.issuer, "https://accounts.google.com");
    }

    #[test]
    fn empty_key_rejected() {
        assert!(OidcProvider::parse_spec("=https://x").is_err());
    }
}
