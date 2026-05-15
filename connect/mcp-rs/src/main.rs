use clap::{Parser, Subcommand};
use ohd_mcp_rs::{http::build_router, open_storage, stdio};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "ohd-mcp-rs", version, about = "OHD MCP server")]
struct Cli {
    /// Storage file path (single-user for v1).
    #[arg(long, env = "OHD_MCP_STORAGE", default_value = "ohd-mcp.ohd")]
    storage: PathBuf,

    #[command(subcommand)]
    transport: Transport,
}

#[derive(Subcommand, Debug)]
enum Transport {
    /// JSON-RPC over stdio — Claude Code / Cursor / Codex local mode.
    Stdio,
    /// JSON-RPC over HTTP — used by mcp.ohd.dev deployment.
    Http {
        #[arg(long, env = "OHD_MCP_BIND", default_value = "0.0.0.0:8445")]
        bind: SocketAddr,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Stdio mode pipes JSON-RPC on stdout — never log there.
    let logger = tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr);
    logger.init();

    let cli = Cli::parse();
    let storage = Arc::new(open_storage(cli.storage)?);

    match cli.transport {
        Transport::Stdio => {
            tracing::info!("ohd-mcp-rs starting in stdio mode");
            stdio::run(storage).await
        }
        Transport::Http { bind } => {
            tracing::info!(addr = %bind, "ohd-mcp-rs listening on HTTP");
            let router = build_router(storage);
            let listener = tokio::net::TcpListener::bind(bind).await?;
            axum::serve(listener, router).await?;
            Ok(())
        }
    }
}
