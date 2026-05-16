//! OHD Identity — the OIDC provider binary. See `idp/SPEC.md`.

use clap::{Parser, Subcommand};
use ohd_idp::{build_router, config, AccountStore, IdpStore, KeyStore};
use std::path::{Path, PathBuf};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "ohd-idp", version, about = "OHD Identity — OpenID Connect provider")]
struct Cli {
    /// Path to `idp.toml`. Individual values can also be overridden with
    /// `OHD_IDP_*` environment variables.
    #[arg(long, env = "OHD_IDP_CONFIG", default_value = "idp.toml")]
    config: PathBuf,

    /// Optional subcommand. With none given, the IdP runs the HTTP server.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Rotate the RS256 signing key: generate a fresh keypair, make it the
    /// active key, and keep the previous public key in `/jwks` for
    /// `keys.rotation_overlap_days` so already-issued `id_token`s still
    /// verify. Run this against a stopped (or about-to-restart) instance —
    /// the server picks the rotated key up on its next start.
    RotateKey,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();
    let cfg = config::load(&cli.config)?;

    std::fs::create_dir_all(&cfg.server.data_dir).ok();

    match cli.command {
        Some(Command::RotateKey) => rotate_key(&cfg),
        None => serve(cfg).await,
    }
}

/// Rotate the signing key and exit — the `rotate-key` subcommand.
fn rotate_key(cfg: &config::Config) -> anyhow::Result<()> {
    let mut keys = KeyStore::load(Path::new(&cfg.keys.signing_key_file))?;
    let (old_kid, new_kid) = keys.rotate(cfg.keys.rotation_overlap_days)?;
    tracing::info!(
        old_kid = %old_kid,
        new_kid = %new_kid,
        overlap_days = cfg.keys.rotation_overlap_days,
        "rotated RS256 signing key"
    );
    println!(
        "signing key rotated: {old_kid} → {new_kid} \
         (old key stays in /jwks for {} days)",
        cfg.keys.rotation_overlap_days
    );
    Ok(())
}

/// Run the HTTP server — the default action.
async fn serve(cfg: config::Config) -> anyhow::Result<()> {
    // Load the RS256 signing keys: the active key (generated on first
    // launch) plus any rotation-overlap keys. Expired overlap keys are
    // dropped here, so `/jwks` only serves keys still inside their window.
    let keys = KeyStore::load(Path::new(&cfg.keys.signing_key_file))?;

    // The shared SaaS account store (email/password credentials). Opening
    // it runs the idempotent `CREATE TABLE IF NOT EXISTS email_credentials`
    // so the IdP works whether or not the SaaS migrated the DB first.
    let accounts = AccountStore::open(&cfg.store.saas_db)?;

    // The IdP-local store — authorization codes, access tokens, sessions.
    let idp_db_path = format!("{}/ohd-idp.db", cfg.server.data_dir.trim_end_matches('/'));
    let idp_store = IdpStore::open(&idp_db_path)?;

    tracing::info!(
        issuer = %cfg.server.issuer,
        kid = %keys.active().kid(),
        overlap_keys = keys.overlap().len(),
        clients = cfg.clients.len(),
        saas_db = %cfg.store.saas_db,
        "ohd-idp configured"
    );

    let listen = cfg.server.listen;
    let router = build_router(cfg, keys, accounts, idp_store);
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(addr = %listen, "ohd-idp listening");
    axum::serve(listener, router).await?;
    Ok(())
}
