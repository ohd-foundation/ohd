use clap::Parser;
use cord_server::{build_router, config, Db};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "cord-server", version, about = "OHD CORD — conversational agent web service")]
struct Cli {
    /// Path to `cord.toml`. Without it the server runs a dev configuration
    /// (loopback listener, dev secrets, no OIDC providers).
    #[arg(long, env = "OHD_CORD_CONFIG")]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let cli = Cli::parse();
    let cfg = match &cli.config {
        Some(path) => config::load(path)?,
        None => {
            tracing::warn!("no --config given — running dev configuration; login is disabled");
            config::Config::dev()
        }
    };

    std::fs::create_dir_all(&cfg.data_dir).ok();
    let db_path = format!("{}/cord.db", cfg.data_dir.trim_end_matches('/'));
    let db = Db::open(&db_path)?;

    let listen = cfg.listen;
    let router = build_router(db, cfg);
    let listener = tokio::net::TcpListener::bind(listen).await?;
    tracing::info!(addr = %listen, "cord-server listening");
    axum::serve(listener, router).await?;
    Ok(())
}
