//! `cord.toml` loader. The file references every secret by environment-
//! variable *name*; [`load`] resolves those names against the process
//! environment so the rest of the program works with plain values.

use anyhow::{anyhow, bail, Context};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::Path;

/// Dev-only fallbacks, used when the server runs with no config file.
const DEV_DATA_KEY: [u8; 32] = [0x0c; 32];
const DEV_SESSION_SECRET: &str = "dev-only-cord-session-secret-replace-me";

/// Fully-resolved runtime configuration. Secrets are real values here,
/// not env-var names.
#[derive(Debug, Clone)]
pub struct Config {
    pub listen: SocketAddr,
    pub public_url: String,
    pub data_dir: String,
    pub session_ttl_hours: i64,
    pub session_secret: String,
    pub data_key: [u8; 32],
    pub providers: Vec<OidcProvider>,
    pub model_providers: Vec<ModelProvider>,
    pub default_model_provider: String,
    pub allow_user_keys: bool,
    pub default_relay: String,
    pub allow_custom_relay: bool,
    /// Directory of the built `cord-web` SPA, if this deployment serves it.
    pub web_dir: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OidcProvider {
    pub id: String,
    /// `"oidc"` (default) or `"dev"`. A `dev` provider logs straight in as
    /// a single fixed identity with no OIDC round-trip — for deployments
    /// that have no IdP wired up yet.
    pub kind: String,
    pub issuer: String,
    pub client_id: String,
    /// Empty for a public (PKCE-only) client.
    pub client_secret: String,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ModelProvider {
    pub id: String,
    pub kind: String,
    pub api_key: String,
    pub models: Vec<String>,
}

impl Config {
    /// In-process dev configuration: loopback listener, dev secrets, no
    /// OIDC providers (login is unavailable until a real config is given).
    pub fn dev() -> Self {
        Self {
            listen: "127.0.0.1:8446".parse().unwrap(),
            public_url: "http://127.0.0.1:8446".to_string(),
            data_dir: ".".to_string(),
            session_ttl_hours: 720,
            session_secret: DEV_SESSION_SECRET.to_string(),
            data_key: DEV_DATA_KEY,
            providers: Vec::new(),
            model_providers: Vec::new(),
            default_model_provider: String::new(),
            allow_user_keys: true,
            default_relay: "https://relay.ohd.dev".to_string(),
            allow_custom_relay: true,
            web_dir: None,
        }
    }

    pub fn provider(&self, id: &str) -> Option<&OidcProvider> {
        self.providers.iter().find(|p| p.id == id)
    }

    pub fn model_provider(&self, id: &str) -> Option<&ModelProvider> {
        self.model_providers.iter().find(|p| p.id == id)
    }
}

/// Load and resolve a `cord.toml`.
pub fn load(path: &Path) -> anyhow::Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading config {}", path.display()))?;
    let file: FileConfig = toml::from_str(&text).context("parsing cord.toml")?;
    resolve(file)
}

fn resolve(file: FileConfig) -> anyhow::Result<Config> {
    let listen: SocketAddr = file
        .server
        .listen
        .parse()
        .with_context(|| format!("server.listen `{}` is not an address", file.server.listen))?;

    let session_secret = env_required(&file.auth.session_jwt_secret_env)?;
    let data_key = parse_data_key(&env_required(&file.server.data_key_env)?)?;

    let providers = file
        .auth
        .provider
        .into_iter()
        .map(|p| {
            let client_secret = match &p.client_secret_env {
                Some(name) => env_optional(name),
                None => String::new(),
            };
            OidcProvider {
                id: p.id,
                kind: if p.kind.is_empty() { "oidc".into() } else { p.kind },
                issuer: p.issuer,
                client_id: p.client_id,
                client_secret,
                scopes: p.scopes,
            }
        })
        .collect();

    let model_providers: Vec<ModelProvider> = file
        .models
        .provider
        .into_iter()
        .map(|p| {
            let api_key = env_optional(&p.api_key_env);
            if api_key.is_empty() {
                tracing::warn!(provider = %p.id, "model provider has no API key set");
            }
            ModelProvider {
                id: p.id,
                kind: p.kind,
                api_key,
                models: p.models,
            }
        })
        .collect();

    let default_model_provider = file
        .models
        .default_provider
        .or_else(|| model_providers.first().map(|p| p.id.clone()))
        .unwrap_or_default();

    Ok(Config {
        listen,
        public_url: file.server.public_url.trim_end_matches('/').to_string(),
        data_dir: file.server.data_dir,
        session_ttl_hours: file.auth.session_ttl_hours,
        session_secret,
        data_key,
        providers,
        model_providers,
        default_model_provider,
        allow_user_keys: file.models.byo.allow_user_keys,
        default_relay: file.relay.default_relay,
        allow_custom_relay: file.relay.allow_custom_relay,
        web_dir: file.server.web_dir,
    })
}

fn env_required(name: &str) -> anyhow::Result<String> {
    let v = std::env::var(name)
        .map_err(|_| anyhow!("required environment variable `{name}` is not set"))?;
    if v.is_empty() {
        bail!("environment variable `{name}` is set but empty");
    }
    Ok(v)
}

fn env_optional(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

fn parse_data_key(b64: &str) -> anyhow::Result<[u8; 32]> {
    let bytes = STANDARD
        .decode(b64.trim())
        .context("data key is not valid base64")?;
    let arr: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| anyhow!("data key must decode to exactly 32 bytes, got {}", bytes.len()))?;
    Ok(arr)
}

// --- TOML file shape -------------------------------------------------------

#[derive(Deserialize)]
struct FileConfig {
    server: FileServer,
    #[serde(default)]
    auth: FileAuth,
    #[serde(default)]
    models: FileModels,
    #[serde(default)]
    relay: FileRelay,
}

#[derive(Deserialize)]
struct FileServer {
    listen: String,
    public_url: String,
    #[serde(default = "d_data_dir")]
    data_dir: String,
    #[serde(default = "d_data_key_env")]
    data_key_env: String,
    #[serde(default)]
    web_dir: Option<String>,
}

#[derive(Deserialize, Default)]
struct FileAuth {
    #[serde(default = "d_session_ttl")]
    session_ttl_hours: i64,
    #[serde(default = "d_session_secret_env")]
    session_jwt_secret_env: String,
    #[serde(default, rename = "provider")]
    provider: Vec<FileOidc>,
}

#[derive(Deserialize)]
struct FileOidc {
    id: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    issuer: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    client_secret_env: Option<String>,
    #[serde(default = "d_scopes")]
    scopes: Vec<String>,
}

#[derive(Deserialize, Default)]
struct FileModels {
    #[serde(default)]
    default_provider: Option<String>,
    #[serde(default, rename = "provider")]
    provider: Vec<FileModelProvider>,
    #[serde(default)]
    byo: FileByo,
}

#[derive(Deserialize)]
struct FileModelProvider {
    id: String,
    kind: String,
    api_key_env: String,
    #[serde(default)]
    models: Vec<String>,
}

#[derive(Deserialize)]
struct FileByo {
    #[serde(default)]
    allow_user_keys: bool,
}

impl Default for FileByo {
    fn default() -> Self {
        Self { allow_user_keys: false }
    }
}

#[derive(Deserialize)]
struct FileRelay {
    #[serde(default = "d_relay")]
    default_relay: String,
    #[serde(default = "d_true")]
    allow_custom_relay: bool,
}

impl Default for FileRelay {
    fn default() -> Self {
        Self {
            default_relay: d_relay(),
            allow_custom_relay: true,
        }
    }
}

fn d_data_dir() -> String {
    "/var/lib/ohd-cord".to_string()
}
fn d_data_key_env() -> String {
    "OHD_CORD_DATA_KEY".to_string()
}
fn d_session_secret_env() -> String {
    "OHD_CORD_SESSION_SECRET".to_string()
}
fn d_session_ttl() -> i64 {
    720
}
fn d_scopes() -> Vec<String> {
    vec!["openid".into(), "email".into(), "profile".into()]
}
fn d_relay() -> String {
    "https://relay.ohd.dev".to_string()
}
fn d_true() -> bool {
    true
}
