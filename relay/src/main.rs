//! OHD Relay — binary entry point.
//!
//! Forwards opaque packets between OHDC consumers and OHD Storage instances
//! behind NAT. See `../SPEC.md` for the implementation contract this crate
//! targets, and `../spec/relay-protocol.md` for the wire spec.

use clap::{Parser, Subcommand};
use ohd_relay::server;

const RELAY_VERSION_BANNER: &str = "OHD Relay v0";

#[derive(Parser, Debug)]
#[command(
    name = "ohd-relay",
    about = "OHD Relay — bridges remote OHDC consumers to storage behind NAT",
    long_about = "OHD Relay forwards opaque ciphertext between OHDC consumers and OHD \
                  Storage instances that can't accept inbound connections (phones, home \
                  servers behind NAT). It does not decrypt; TLS is end-to-end through the \
                  tunnel. See SPEC.md for the implementation contract.",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Start the relay (HTTP server with WebSocket tunnel + attach paths).
    Serve {
        /// Path to a `relay.toml` config file (currently informational).
        #[arg(long, value_name = "PATH", default_value = "relay.toml")]
        config: String,

        /// Bind address for the relay's HTTP listener (overrides --port).
        #[arg(long, value_name = "ADDR")]
        bind: Option<String>,

        /// SQLite database path for the registration table.
        #[arg(long, value_name = "PATH", default_value = "/tmp/ohd-relay.db")]
        db: String,

        /// Bind port (used when --bind is not provided).
        #[arg(long, default_value_t = 8443)]
        port: u16,

        /// Optional UDP/QUIC listen address for HTTP/3. When set, the
        /// relay runs an in-binary HTTP/3 listener for the REST endpoints
        /// alongside the HTTP/2 listener. WebSocket-based tunnel/attach
        /// paths stay on HTTP/2 (RFC 9220 immaturity in `h3`).
        #[arg(long, value_name = "ADDR")]
        http3_listen: Option<std::net::SocketAddr>,

        /// PEM-encoded certificate chain for the HTTP/3 listener (one or
        /// more `CERTIFICATE` blocks). Must be paired with `--http3-key`.
        /// When omitted, a self-signed dev cert is generated and a
        /// warning is printed to stderr.
        #[arg(long, value_name = "PATH", requires = "http3_key")]
        http3_cert: Option<std::path::PathBuf>,

        /// PEM-encoded private key for the HTTP/3 listener (PKCS#8,
        /// PKCS#1, or SEC1). Must be paired with `--http3-cert`.
        #[arg(long, value_name = "PATH", requires = "http3_cert")]
        http3_key: Option<std::path::PathBuf>,

        /// Optional UDP listen address for the **raw QUIC tunnel** (ALPN
        /// `ohd-tnl1`). Storage processes that prefer connection-migration-
        /// friendly transports dial here instead of the HTTP/2 WebSocket
        /// tunnel. When omitted, only the HTTP/2 WebSocket tunnel is
        /// available. See `src/quic_tunnel.rs` for the wire shape.
        #[arg(long, value_name = "ADDR")]
        quic_tunnel_listen: Option<std::net::SocketAddr>,

        /// PEM-encoded certificate chain for the raw QUIC tunnel listener.
        /// Must be paired with `--quic-tunnel-key`. When omitted, the
        /// listener falls back to a dev self-signed cert and emits a
        /// stderr warning, matching the `--http3-cert` story.
        #[arg(long, value_name = "PATH", requires = "quic_tunnel_key")]
        quic_tunnel_cert: Option<std::path::PathBuf>,

        /// PEM-encoded private key for the raw QUIC tunnel listener.
        /// Must be paired with `--quic-tunnel-cert`.
        #[arg(long, value_name = "PATH", requires = "quic_tunnel_cert")]
        quic_tunnel_key: Option<std::path::PathBuf>,
    },

    /// Print a one-line health status and exit.
    Health,

    /// Print version information and exit.
    Version,
}

fn main() -> anyhow::Result<()> {
    // Initialize tracing only when needed; health/version stay quiet.
    let cli = Cli::parse();

    match cli.command {
        Command::Health => {
            println!("{} \u{2014} health: ok", RELAY_VERSION_BANNER);
            Ok(())
        }
        Command::Version => {
            println!(
                "{} ({} v{})",
                RELAY_VERSION_BANNER,
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            );
            Ok(())
        }
        Command::Serve {
            config,
            bind,
            db,
            port,
            http3_listen,
            http3_cert,
            http3_key,
            quic_tunnel_listen,
            quic_tunnel_cert,
            quic_tunnel_key,
        } => {
            init_tracing();
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(server::run_serve(server::ServeOptions {
                config_path: config,
                bind_override: bind,
                db_path: db,
                port,
                http3_listen,
                http3_cert,
                http3_key,
                quic_tunnel_listen,
                quic_tunnel_cert,
                quic_tunnel_key,
            }))
        }
    }
}

fn init_tracing() {
    use tracing_subscriber::{fmt, EnvFilter};
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("ohd_relay=info,axum=warn"));
    let _ = fmt().with_env_filter(filter).try_init();
}
