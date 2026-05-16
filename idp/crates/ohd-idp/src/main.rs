//! OHD Identity — the OIDC provider binary. See `idp/SPEC.md`.

use clap::Parser;
use ohd_idp::{build_router, config, SigningKey};
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
    tracing::info!(
        issuer = %cfg.server.issuer,
        kid = %signing_key.kid(),
        clients = cfg.clients.len(),
        "ohd-idp configured"
    );

    let listen = cfg.server.listen;
    let router = build_router(cfg, signing_key);
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(addr = %listen, "ohd-idp listening");
    axum::serve(listener, router).await?;
    Ok(())
}
