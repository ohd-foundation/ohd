//! `idp.toml` loader. The file references every client secret by
//! environment-variable *name*; [`load`] resolves those names against the
//! process environment so the rest of the program works with plain
//! values. Mirrors the `cord.toml` / `relay.toml` config pattern.
//!
//! Individual scalar fields can additionally be overridden directly by an
//! `OHD_IDP_*` environment variable — see [`apply_env_overrides`].

use anyhow::{bail, Context};
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::Path;

/// Fully-resolved runtime configuration. Client secrets are real values
/// here, not env-var names.
#[derive(Debug, Clone)]
pub struct Config {
    pub server: ServerConfig,
    pub store: StoreConfig,
    pub keys: KeysConfig,
    pub session: SessionConfig,
    pub signup: SignupConfig,
    pub recovery: RecoveryConfig,
    pub clients: Vec<ClientConfig>,
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub listen: SocketAddr,
    /// The OIDC `iss` — must be exact. No trailing slash.
    pub issuer: String,
    pub data_dir: String,
}

#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// v1: the shared OHD SaaS SQLite database. Not opened in Phase 1.
    pub saas_db: String,
}

#[derive(Debug, Clone)]
pub struct KeysConfig {
    /// RS256 signing key, PEM-encoded; generated + persisted on first
    /// launch, published at `/jwks`.
    pub signing_key_file: String,
    /// How long a rotated-out public key stays in the JWKS.
    pub rotation_overlap_days: i64,
}

#[derive(Debug, Clone)]
pub struct SessionConfig {
    /// Browser SSO session lifetime at the IdP.
    pub sso_ttl_hours: i64,
    /// OHD authorization-code lifetime.
    pub code_ttl_secs: i64,
}

#[derive(Debug, Clone)]
pub struct SignupConfig {
    /// Allow self-service email/password sign-up.
    pub open: bool,
}

#[derive(Debug, Clone)]
pub struct RecoveryConfig {
    /// Allow "sign in with a recovery code".
    pub enabled: bool,
}

/// A registered relying party.
#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub id: String,
    /// Exact-matched at `/authorize`; no wildcards.
    pub redirect_uris: Vec<String>,
    /// `true` for a public (PKCE-only) client with no secret.
    pub public: bool,
    /// Empty for a public client; otherwise the resolved secret value.
    pub client_secret: String,
}

impl Config {
    /// Look up a registered client by `client_id`.
    pub fn client(&self, id: &str) -> Option<&ClientConfig> {
        self.clients.iter().find(|c| c.id == id)
    }
}

/// Load and resolve an `idp.toml`, then apply any `OHD_IDP_*` scalar
/// environment overrides.
pub fn load(path: &Path) -> anyhow::Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading config {}", path.display()))?;
    let file: FileConfig = toml::from_str(&text).context("parsing idp.toml")?;
    let mut config = resolve(file)?;
    apply_env_overrides(&mut config)?;
    Ok(config)
}

/// Parse `idp.toml` text directly (without touching the filesystem) and
/// resolve it. Env overrides are applied. Useful for tests.
pub fn from_str(text: &str) -> anyhow::Result<Config> {
    let file: FileConfig = toml::from_str(text).context("parsing idp.toml")?;
    let mut config = resolve(file)?;
    apply_env_overrides(&mut config)?;
    Ok(config)
}

fn resolve(file: FileConfig) -> anyhow::Result<Config> {
    let listen: SocketAddr = file
        .server
        .listen
        .parse()
        .with_context(|| format!("server.listen `{}` is not an address", file.server.listen))?;

    let clients = file
        .client
        .into_iter()
        .map(|c| {
            let client_secret = match (&c.client_secret_env, c.public) {
                (Some(name), false) => env_optional(name),
                _ => String::new(),
            };
            ClientConfig {
                id: c.id,
                redirect_uris: c.redirect_uris,
                public: c.public,
                client_secret,
            }
        })
        .collect();

    Ok(Config {
        server: ServerConfig {
            listen,
            issuer: file.server.issuer.trim_end_matches('/').to_string(),
            data_dir: file.server.data_dir,
        },
        store: StoreConfig {
            saas_db: file.store.saas_db,
        },
        keys: KeysConfig {
            signing_key_file: file.keys.signing_key_file,
            rotation_overlap_days: file.keys.rotation_overlap_days,
        },
        session: SessionConfig {
            sso_ttl_hours: file.session.sso_ttl_hours,
            code_ttl_secs: file.session.code_ttl_secs,
        },
        signup: SignupConfig {
            open: file.signup.open,
        },
        recovery: RecoveryConfig {
            enabled: file.recovery.enabled,
        },
        clients,
    })
}

/// Apply direct `OHD_IDP_*` scalar overrides on top of the file values.
/// This lets a deployment tweak a single setting without re-templating
/// the whole `idp.toml` — the same convenience `cord` / `relay` offer.
fn apply_env_overrides(config: &mut Config) -> anyhow::Result<()> {
    if let Some(v) = env_var("OHD_IDP_LISTEN") {
        config.server.listen = v
            .parse()
            .with_context(|| format!("OHD_IDP_LISTEN `{v}` is not an address"))?;
    }
    if let Some(v) = env_var("OHD_IDP_ISSUER") {
        config.server.issuer = v.trim_end_matches('/').to_string();
    }
    if let Some(v) = env_var("OHD_IDP_DATA_DIR") {
        config.server.data_dir = v;
    }
    if let Some(v) = env_var("OHD_IDP_STORE_SAAS_DB") {
        config.store.saas_db = v;
    }
    if let Some(v) = env_var("OHD_IDP_SIGNING_KEY_FILE") {
        config.keys.signing_key_file = v;
    }
    if let Some(v) = env_var("OHD_IDP_ROTATION_OVERLAP_DAYS") {
        config.keys.rotation_overlap_days = v
            .parse()
            .with_context(|| format!("OHD_IDP_ROTATION_OVERLAP_DAYS `{v}` is not an integer"))?;
    }
    if let Some(v) = env_var("OHD_IDP_SSO_TTL_HOURS") {
        config.session.sso_ttl_hours = v
            .parse()
            .with_context(|| format!("OHD_IDP_SSO_TTL_HOURS `{v}` is not an integer"))?;
    }
    if let Some(v) = env_var("OHD_IDP_CODE_TTL_SECS") {
        config.session.code_ttl_secs = v
            .parse()
            .with_context(|| format!("OHD_IDP_CODE_TTL_SECS `{v}` is not an integer"))?;
    }
    if let Some(v) = env_var("OHD_IDP_SIGNUP_OPEN") {
        config.signup.open = parse_bool("OHD_IDP_SIGNUP_OPEN", &v)?;
    }
    if let Some(v) = env_var("OHD_IDP_RECOVERY_ENABLED") {
        config.recovery.enabled = parse_bool("OHD_IDP_RECOVERY_ENABLED", &v)?;
    }
    Ok(())
}

/// Read an env var, treating an empty value as unset.
fn env_var(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => None,
    }
}

fn env_optional(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

fn parse_bool(name: &str, v: &str) -> anyhow::Result<bool> {
    match v {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        other => bail!("{name} `{other}` is not a boolean (expected true/false)"),
    }
}

// --- TOML file shape -------------------------------------------------------

#[derive(Deserialize)]
struct FileConfig {
    server: FileServer,
    #[serde(default)]
    store: FileStore,
    #[serde(default)]
    keys: FileKeys,
    #[serde(default)]
    session: FileSession,
    #[serde(default)]
    signup: FileSignup,
    #[serde(default)]
    recovery: FileRecovery,
    #[serde(default, rename = "client")]
    client: Vec<FileClient>,
}

#[derive(Deserialize)]
struct FileServer {
    listen: String,
    issuer: String,
    #[serde(default = "d_data_dir")]
    data_dir: String,
}

#[derive(Deserialize)]
struct FileStore {
    #[serde(default = "d_saas_db")]
    saas_db: String,
}

impl Default for FileStore {
    fn default() -> Self {
        Self { saas_db: d_saas_db() }
    }
}

#[derive(Deserialize)]
struct FileKeys {
    #[serde(default = "d_signing_key_file")]
    signing_key_file: String,
    #[serde(default = "d_rotation_overlap_days")]
    rotation_overlap_days: i64,
}

impl Default for FileKeys {
    fn default() -> Self {
        Self {
            signing_key_file: d_signing_key_file(),
            rotation_overlap_days: d_rotation_overlap_days(),
        }
    }
}

#[derive(Deserialize)]
struct FileSession {
    #[serde(default = "d_sso_ttl_hours")]
    sso_ttl_hours: i64,
    #[serde(default = "d_code_ttl_secs")]
    code_ttl_secs: i64,
}

impl Default for FileSession {
    fn default() -> Self {
        Self {
            sso_ttl_hours: d_sso_ttl_hours(),
            code_ttl_secs: d_code_ttl_secs(),
        }
    }
}

#[derive(Deserialize)]
struct FileSignup {
    #[serde(default = "d_true")]
    open: bool,
}

impl Default for FileSignup {
    fn default() -> Self {
        Self { open: d_true() }
    }
}

#[derive(Deserialize)]
struct FileRecovery {
    #[serde(default = "d_true")]
    enabled: bool,
}

impl Default for FileRecovery {
    fn default() -> Self {
        Self { enabled: d_true() }
    }
}

#[derive(Deserialize)]
struct FileClient {
    id: String,
    #[serde(default)]
    redirect_uris: Vec<String>,
    #[serde(default)]
    public: bool,
    #[serde(default)]
    client_secret_env: Option<String>,
}

fn d_data_dir() -> String {
    "/var/lib/ohd-idp".to_string()
}
fn d_saas_db() -> String {
    "/var/lib/ohd-saas/ohd-saas.db".to_string()
}
fn d_signing_key_file() -> String {
    "/var/lib/ohd-idp/signing-key.pem".to_string()
}
fn d_rotation_overlap_days() -> i64 {
    7
}
fn d_sso_ttl_hours() -> i64 {
    12
}
fn d_code_ttl_secs() -> i64 {
    120
}
fn d_true() -> bool {
    true
}
