//! OHD Identity — the OIDC provider binary. See `idp/SPEC.md`.

use clap::Parser;
use ohd_idp::{build_router, config, AccountStore, IdpStore, SigningKey};
use std::path::{Path, PathBuf};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "ohd-idp", version, about = "OHD Identity — OpenID Connect provider")]
struct Cli {
    /// Path to `idp.toml`. Individual values can also be overridden with
    /// `OHD_IDP_*` environment variables.
    #[arg(long, env = "OHD_IDP_CONFIG", default_value = "idp.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();
    let cfg = config::load(&cli.config)?;

    std::fs::create_dir_all(&cfg.server.data_dir).ok();

    // Generate the RS256 signing key on first launch, reuse it after —
    // the `kid` stays stable, so previously-issued `id_token`s verify.
    let signing_key = SigningKey::load_or_generate(Path::new(&cfg.keys.signing_key_file))?;

    // The shared SaaS account store (email/password credentials). Opening
    // it runs the idempotent `CREATE TABLE IF NOT EXISTS email_credentials`
    // so the IdP works whether or not the SaaS migrated the DB first.
    let accounts = AccountStore::open(&cfg.store.saas_db)?;

    // The IdP-local store — authorization codes + access tokens.
    let idp_db_path = format!("{}/ohd-idp.db", cfg.server.data_dir.trim_end_matches('/'));
    let idp_store = IdpStore::open(&idp_db_path)?;

    tracing::info!(
        issuer = %cfg.server.issuer,
        kid = %signing_key.kid(),
        clients = cfg.clients.len(),
        saas_db = %cfg.store.saas_db,
        "ohd-idp configured"
    );

    let listen = cfg.server.listen;
    let router = build_router(cfg, signing_key, accounts, idp_store);
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(addr = %listen, "ohd-idp listening");
    axum::serve(listener, router).await?;
    Ok(())
}
