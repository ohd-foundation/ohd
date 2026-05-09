//! `relay.toml` configuration loading.
//!
//! The config is operator-facing — loaded at process start, not reloaded.
//! Restart-to-reconfigure is fine for the relay's deploy cadence.
//!
//! ## Shape (see `deploy/relay.example.toml` for the full annotated example):
//!
//! ```toml
//! # Optional. Public hostname used to compose `rendezvous_url` in
//! # register responses. When omitted, the relay derives it from the
//! # bind address (which is a dev affordance — set this in production).
//! public_host = "relay.example.com"
//!
//! [push.fcm]
//! project_id = "ohd-cloud"
//! service_account_path = "/run/secrets/fcm_service_account.json"
//!
//! [push.apns]
//! team_id = "ABC123"
//! key_id = "DEF456"
//! key_path = "/run/secrets/apns_key.p8"
//! bundle_id = "com.ohd.connect"
//! environment = "production"  # or "sandbox"
//!
//! [authority]   # Only effective with `--features authority`
//! enabled = true
//! fulcio_url = "https://fulcio.openhealth-data.org"
//! oidc_idp_url = "https://idp.openhealth-data.org"
//! org_label = "EMS Prague Region"
//! org_country = "CZ"
//! rekor_url = "https://rekor.openhealth-data.org"
//!
//! [auth.registration]
//! # Optional OIDC issuer allowlist. When unset/empty, the relay accepts
//! # any registration (legacy behavior). When set, storage must present a
//! # valid `id_token` from one of the listed issuers.
//! allowed_issuers = [
//!   { issuer = "https://accounts.ohd.org",    expected_audience = "ohd-relay-cloud" },
//!   { issuer = "https://accounts.google.com", expected_audience = "ohd-relay-cloud" },
//! ]
//! jwks_cache_ttl_secs = 3600
//! require_oidc        = false
//! ```
//!
//! Missing sections are no-ops: a relay without `[push.fcm]` simply
//! can't deliver FCM pushes. Misconfiguration (e.g. unreadable service
//! account file at boot) is a hard error so operators see it during
//! deploy, not at the first real wake-up.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Top-level config loaded from `relay.toml`. Every field is optional so
/// `relay.toml` itself can be empty (legacy CLI-only operation works).
#[derive(Debug, Default, Clone, Deserialize)]
pub struct RelayConfig {
    pub public_host: Option<String>,
    #[serde(default)]
    pub push: PushConfig,
    #[serde(default)]
    pub authority: AuthorityConfig,
    #[serde(default)]
    pub auth: AuthConfig,
}

// ---------------------------------------------------------------------------
// [auth.registration] — per-OIDC gating for storage registration
// ---------------------------------------------------------------------------

/// Top-level `[auth]` section. Currently only carries
/// `[auth.registration]`; future blocks (e.g. `[auth.consumer]`) can slot
/// in alongside without churning the config schema.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub registration: RegistrationAuthConfig,
}

/// `[auth.registration]` — controls who is allowed to register storage
/// instances with this relay.
///
/// **Default (unset / empty `allowed_issuers`)**: permissive. Anyone who
/// can reach the relay can register. Backwards-compatible.
///
/// **Configured (`allowed_issuers` set)**: storage must present an
/// `id_token` from one of the listed OIDC issuers. The relay verifies
/// signature, exp, aud (`expected_audience` from this config), nbf, iat
/// against the issuer's JWKS, and records `(iss, sub)` alongside the
/// registration as the operator-side identity for audit.
#[derive(Debug, Clone, Deserialize)]
pub struct RegistrationAuthConfig {
    /// OIDC issuers accepted for registration. Empty list = permissive
    /// (any caller may register, no id_token required).
    #[serde(default)]
    pub allowed_issuers: Vec<AllowedIssuer>,

    /// JWKS cache TTL in seconds. Default 3600 (1h). Even within the TTL,
    /// the verifier transparently refreshes when it sees a `kid` it does
    /// not have cached (key-rotation friendliness).
    #[serde(default = "default_jwks_cache_ttl_secs")]
    pub jwks_cache_ttl_secs: u64,

    /// When true: registrations from operators that DON'T present an
    /// id_token are rejected with `OIDC_REQUIRED`. When false (default),
    /// permissive registration still works even when `allowed_issuers` is
    /// set — but if a token IS presented it is verified.
    #[serde(default)]
    pub require_oidc: bool,
}

impl Default for RegistrationAuthConfig {
    fn default() -> Self {
        Self {
            allowed_issuers: Vec::new(),
            jwks_cache_ttl_secs: default_jwks_cache_ttl_secs(),
            require_oidc: false,
        }
    }
}

/// One entry in the `allowed_issuers` allowlist.
#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct AllowedIssuer {
    /// OIDC issuer URL. Matched **exactly** against the JWT's `iss`
    /// claim. The OpenID Provider's discovery document is fetched at
    /// `<issuer>/.well-known/openid-configuration`.
    pub issuer: String,

    /// Required `aud` claim value. The relay rejects tokens whose `aud`
    /// does not include this string. Operators typically register a
    /// dedicated audience per relay (e.g. `"ohd-relay-cloud"`).
    pub expected_audience: String,
}

fn default_jwks_cache_ttl_secs() -> u64 {
    3600
}

impl RegistrationAuthConfig {
    /// True when operators must authenticate via OIDC to register. False
    /// = permissive (no allowlist) or "soft" (allowlist set but
    /// `require_oidc=false`).
    pub fn is_gated(&self) -> bool {
        !self.allowed_issuers.is_empty()
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct PushConfig {
    pub fcm: Option<FcmConfigSection>,
    pub apns: Option<ApnsConfigSection>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FcmConfigSection {
    pub project_id: String,
    pub service_account_path: PathBuf,
    /// Override for the FCM messages:send base URL. Tests / private
    /// FCM mirrors only.
    #[serde(default)]
    pub fcm_base_url: Option<String>,
    /// Override for the OAuth2 token URL. Tests only.
    #[serde(default)]
    pub token_base_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApnsConfigSection {
    pub team_id: String,
    pub key_id: String,
    pub key_path: PathBuf,
    pub bundle_id: String,
    /// `"production"` (default) or `"sandbox"`.
    #[serde(default = "default_apns_env")]
    pub environment: String,
    #[serde(default)]
    pub override_base_url: Option<String>,
}

fn default_apns_env() -> String {
    "production".into()
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct AuthorityConfig {
    /// Master switch. Even with `--features authority`, the relay only
    /// runs in authority mode if this is `true` and the section is fully
    /// populated.
    #[serde(default)]
    pub enabled: bool,
    pub fulcio_url: Option<String>,
    pub oidc_idp_url: Option<String>,
    pub rekor_url: Option<String>,
    /// Subject CN for the issued cert: e.g. `"EMS Prague Region"`.
    pub org_label: Option<String>,
    /// ISO 3166-1 alpha-2 country code: `"CZ"`, `"DE"`, etc.
    pub org_country: Option<String>,
    /// Path to the OIDC ID-token file (mounted by the deployment system).
    /// The file is re-read on every refresh; rotation is handled outside
    /// this binary.
    pub oidc_id_token_path: Option<PathBuf>,
    /// Optional override for cert validity in seconds (default 24h).
    #[serde(default)]
    pub cert_validity_seconds: Option<u64>,
}

impl RelayConfig {
    /// Load from disk. Returns `Default::default()` (empty) when the path
    /// doesn't exist, so the legacy CLI-only path keeps working when no
    /// `relay.toml` is supplied.
    pub fn load(path: impl AsRef<Path>) -> anyhow::Result<Self> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("read {}: {}", path.display(), e))?;
        let parsed: Self = toml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("parse {}: {}", path.display(), e))?;
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn empty_path_returns_default() {
        let cfg = RelayConfig::load("/nonexistent/relay.toml").unwrap();
        assert!(cfg.public_host.is_none());
        assert!(cfg.push.fcm.is_none());
        assert!(cfg.push.apns.is_none());
        assert!(!cfg.authority.enabled);
    }

    #[test]
    fn fcm_section_roundtrips() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
            [push.fcm]
            project_id = "ohd-cloud"
            service_account_path = "/run/secrets/fcm.json"
            "#
        )
        .unwrap();
        let cfg = RelayConfig::load(f.path()).unwrap();
        let fcm = cfg.push.fcm.unwrap();
        assert_eq!(fcm.project_id, "ohd-cloud");
        assert_eq!(
            fcm.service_account_path,
            PathBuf::from("/run/secrets/fcm.json")
        );
    }

    #[test]
    fn apns_section_with_environment_default() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
            [push.apns]
            team_id = "TID"
            key_id = "KID"
            key_path = "/run/secrets/apns.p8"
            bundle_id = "com.ohd.connect"
            "#
        )
        .unwrap();
        let cfg = RelayConfig::load(f.path()).unwrap();
        let apns = cfg.push.apns.unwrap();
        assert_eq!(apns.environment, "production");
        assert_eq!(apns.bundle_id, "com.ohd.connect");
    }

    #[test]
    fn authority_section_parses_when_present() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
            [authority]
            enabled = true
            fulcio_url = "https://fulcio.example"
            oidc_idp_url = "https://idp.example"
            org_label = "EMS Prague"
            org_country = "CZ"
            "#
        )
        .unwrap();
        let cfg = RelayConfig::load(f.path()).unwrap();
        assert!(cfg.authority.enabled);
        assert_eq!(cfg.authority.fulcio_url.as_deref(), Some("https://fulcio.example"));
        assert_eq!(cfg.authority.org_country.as_deref(), Some("CZ"));
    }

    #[test]
    fn registration_auth_defaults_permissive() {
        let cfg = RelayConfig::default();
        assert!(!cfg.auth.registration.is_gated());
        assert!(!cfg.auth.registration.require_oidc);
        // Manual Default impl applies the same TTL the serde default uses.
        assert_eq!(cfg.auth.registration.jwks_cache_ttl_secs, 3600);
    }

    #[test]
    fn registration_auth_section_parses() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
            [auth.registration]
            allowed_issuers = [
              {{ issuer = "https://accounts.ohd.org",    expected_audience = "ohd-relay-cloud" }},
              {{ issuer = "https://accounts.google.com", expected_audience = "ohd-relay-cloud" }},
            ]
            jwks_cache_ttl_secs = 7200
            require_oidc = true
            "#
        )
        .unwrap();
        let cfg = RelayConfig::load(f.path()).unwrap();
        assert!(cfg.auth.registration.is_gated());
        assert!(cfg.auth.registration.require_oidc);
        assert_eq!(cfg.auth.registration.jwks_cache_ttl_secs, 7200);
        assert_eq!(cfg.auth.registration.allowed_issuers.len(), 2);
        assert_eq!(
            cfg.auth.registration.allowed_issuers[0].issuer,
            "https://accounts.ohd.org"
        );
        assert_eq!(
            cfg.auth.registration.allowed_issuers[0].expected_audience,
            "ohd-relay-cloud"
        );
    }

    #[test]
    fn registration_auth_jwks_default_when_block_present() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(
            f,
            r#"
            [auth.registration]
            allowed_issuers = [
              {{ issuer = "https://idp.example", expected_audience = "rly" }},
            ]
            "#
        )
        .unwrap();
        let cfg = RelayConfig::load(f.path()).unwrap();
        // serde default kicks in when the field is absent under the block.
        assert_eq!(cfg.auth.registration.jwks_cache_ttl_secs, 3600);
        assert!(!cfg.auth.registration.require_oidc);
    }

    #[test]
    fn public_host_at_top_level() {
        let mut f = NamedTempFile::new().unwrap();
        writeln!(f, r#"public_host = "relay.example.com""#).unwrap();
        let cfg = RelayConfig::load(f.path()).unwrap();
        assert_eq!(cfg.public_host.as_deref(), Some("relay.example.com"));
    }
}
