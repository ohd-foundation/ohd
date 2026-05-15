use clap::Parser;
use ohd_saas::{build_router, Config, Db};
use std::net::SocketAddr;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "ohd-saas", version, about = "OHD account & billing service")]
struct Cli {
    /// SQLite database path. Created if missing.
    #[arg(long, env = "OHD_SAAS_DB", default_value = "ohd-saas.db")]
    db: String,

    /// Bind address.
    #[arg(long, env = "OHD_SAAS_BIND", default_value = "0.0.0.0:8444")]
    bind: SocketAddr,

    /// HS256 JWT secret. **Required in production** — defaults to a
    /// dev-mode literal so local smoke tests work out of the box.
    #[arg(long, env = "OHD_SAAS_JWT_SECRET", default_value = "dev-only-replace-me")]
    jwt_secret: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let cli = Cli::parse();
    let db = Db::open(&cli.db)?;
    db.migrate()?;
    let config = Config {
        jwt_secret: cli.jwt_secret,
        token_ttl_days: 90,
    };
    let router = build_router(db, config);
    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    tracing::info!(addr = %cli.bind, "ohd-saas listening");
    axum::serve(listener, router).await?;
    Ok(())
}
