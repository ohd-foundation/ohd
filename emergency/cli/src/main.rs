//! `ohd-emergency` — operator-side CLI for OHD Emergency deployments.
//!
//! Real bodies (v0.1):
//!
//! | Subcommand | Backing |
//! |---|---|
//! | `login` | Writes the encrypted config vault at `~/.config/ohd-emergency/config.toml` (mode 0600). |
//! | `oidc-login --issuer URL --client-id ID` | OAuth 2.0 Device Authorization Grant (RFC 8628) against any compliant issuer. Mirrors `connect/cli`'s implementation. |
//! | `logout` | Drops tokens from the vault (preserves storage URL + station label). |
//! | `cert info` | Reads the configured authority cert PEM, prints subject / issuer / validity / SHA-256 fingerprint. |
//! | `cert refresh` / `cert rotate` | TBD until Fulcio integration lands; print informative pointers. |
//! | `roster {list,add,remove,status}` | Operator-side TOML at `$XDG_DATA_HOME/ohd-emergency/roster.toml` (or `config.roster_path`). NOT OHDC. |
//! | `audit list` / `audit export` | Calls `OhdcService.AuditQuery`. Storage's handler is `Unimplemented` today; the CLI surfaces the error cleanly. |
//! | `case-export` | Calls `OhdcService.GetCase` + `OhdcService.QueryEvents` + (best-effort) `OhdcService.AuditQuery`. Writes a portable JSON archive (`ohd-emergency.case-export.v1`). |
//!
//! ## Vault / KMS
//!
//! The on-disk config file is encrypted at rest by default. The
//! `--kms-backend auto|keyring|passphrase|none` global flag picks the
//! backend; `auto` (default) tries the OS keyring (Linux Secret Service
//! / macOS Keychain / Windows Credential Manager) and falls back to a
//! passphrase-derived AES-GCM key (Argon2id KDF) on headless machines.
//! The passphrase can be supplied via the `OHD_EMERGENCY_VAULT_PASSPHRASE`
//! env var for unattended (CI / Docker) operation. Legacy plaintext-TOML
//! configs still load (back-compat).
//!
//! See `README.md` for argument shapes + examples.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};

mod audit;
mod case_export;
mod cert;
mod client;
mod config;
mod kms;
mod roster;

// OAuth2 device-flow lives in the shared `ohd-cli-auth` crate (workspace
// path: storage/crates/ohd-cli-auth). Aliased so call sites read `oidc::`
// unchanged. The same crate's `ulid` module is imported directly in
// `audit.rs` + `case_export.rs`.
use ohd_cli_auth::oidc;

/// Generated Connect-RPC client stubs. Produced at build time by
/// `connectrpc-build` from `../../storage/proto/ohdc/v0/ohdc.proto`.
pub mod proto {
    connectrpc::include_generated!();
}

use crate::client::OhdcClient;
use crate::config::Config;

/// Operator CLI for OHD Emergency.
///
/// Manages the operator's authority cert lifecycle, responder roster,
/// audit log queries, and case archive exports.
#[derive(Debug, Parser)]
#[command(name = "ohd-emergency", version, about, long_about = None)]
struct Cli {
    /// Override the config-file storage URL. Useful when targeting a
    /// throwaway local server during development.
    #[arg(long, global = true)]
    storage: Option<String>,

    /// Override the config-file token. Useful for one-off calls.
    #[arg(long, global = true)]
    token: Option<String>,

    /// Skip TLS server-cert verification when speaking HTTP/3 over a
    /// `https+h3://` URL. Dev-only; required when the server uses a
    /// self-signed cert (no real CA chain). Production HTTP/3
    /// deployments should use a public-CA-issued cert and omit this.
    #[arg(long, global = true)]
    insecure_skip_verify: bool,

    /// KMS backend for the credential vault. `auto` tries the OS
    /// keyring (Linux Secret Service / macOS Keychain / Windows
    /// Credential Manager) and falls back to a passphrase-derived
    /// AES-GCM key (Argon2id KDF) for headless environments.
    #[arg(long, global = true, default_value = "auto")]
    kms_backend: String,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Save (storage_url, token, station_label, ...) to the encrypted
    /// vault at `~/.config/ohd-emergency/config.toml`.
    Login(LoginArgs),

    /// Run OAuth 2.0 Device Authorization Grant against the operator's
    /// IdP. Mirrors `ohd-connect oidc-login`.
    OidcLogin(OidcLoginArgs),

    /// Drop tokens from the vault (keeps storage URL, station label,
    /// authority-cert path, roster path).
    Logout,

    /// Authority-cert lifecycle (refresh, inspect, rotate).
    #[command(subcommand)]
    Cert(CertCmd),

    /// Responder roster management (operator-side state, NOT OHDC).
    #[command(subcommand)]
    Roster(RosterCmd),

    /// Operator-side audit log queries.
    #[command(subcommand)]
    Audit(AuditCmd),

    /// Export a case archive (events + audit) for legal / regulatory review.
    CaseExport(CaseExportArgs),
}

// ---- login --------------------------------------------------------------

#[derive(Debug, Args)]
struct LoginArgs {
    /// OHDC endpoint, e.g. `http://localhost:8443` (h2c) or
    /// `https+h3://localhost:18443` (HTTP/3).
    #[arg(long)]
    storage: String,

    /// Operator-side bearer token (`ohds_…` for self-session, or whatever
    /// the relay issues for the operator).
    #[arg(long)]
    token: String,

    /// Free-form station label (e.g. "EMS Prague Region"). Surfaced in
    /// `cert info` + the case-export archive header.
    #[arg(long)]
    station_label: Option<String>,

    /// Path to the operator's authority cert PEM. `cert info` reads this.
    #[arg(long)]
    authority_cert: Option<PathBuf>,

    /// Override the operator-side roster TOML path.
    #[arg(long)]
    roster_path: Option<PathBuf>,
}

// ---- oidc-login ---------------------------------------------------------

#[derive(Debug, Args)]
struct OidcLoginArgs {
    /// OIDC / OAuth issuer URL. Discovered via .well-known. Typical
    /// values: the operator's Keycloak / Authentik / Auth0 instance,
    /// or a managed IdP (Google, Microsoft, etc.).
    #[arg(long)]
    issuer: String,

    /// OAuth client_id registered with the issuer for this CLI.
    #[arg(long, default_value = "ohd-emergency-cli")]
    client_id: String,

    /// OAuth/OIDC scopes (space-separated).
    #[arg(long, default_value = "openid offline_access")]
    scope: String,

    /// Storage URL to record alongside the tokens. Defaults to the
    /// existing config's storage_url, then to the issuer URL.
    #[arg(long)]
    storage: Option<String>,
}

// ---- cert ---------------------------------------------------------------

#[derive(Debug, Subcommand)]
enum CertCmd {
    /// Print the configured authority cert (subject, issuer, validity,
    /// SHA-256 fingerprint).
    Info,
    /// Trigger a manual refresh of the org's daily Fulcio-issued cert.
    Refresh,
    /// Rotate the org's daily-refresh keypair.
    Rotate,
}

// ---- roster -------------------------------------------------------------

#[derive(Debug, Subcommand)]
enum RosterCmd {
    /// List all responders in the operator-side roster.
    List(RosterPathArg),

    /// Add a responder to the operator-side roster.
    Add {
        #[command(flatten)]
        path: RosterPathArg,
        /// Free-form label, e.g. "Dr.Smith" or "Ambulance#7".
        #[arg(long)]
        label: String,
        /// Role: `responder` (paramedic / field crew) or `dispatcher`.
        #[arg(long, default_value = "responder")]
        role: String,
    },

    /// Remove a responder from the operator-side roster.
    Remove {
        #[command(flatten)]
        path: RosterPathArg,
        /// Label that uniquely identifies the responder.
        #[arg(long)]
        label: String,
    },

    /// Show on-duty / total counts.
    Status(RosterPathArg),
}

#[derive(Debug, Args)]
struct RosterPathArg {
    /// Override the roster TOML path. Defaults to
    /// `$XDG_DATA_HOME/ohd-emergency/roster.toml` or `config.roster_path`.
    #[arg(long)]
    roster_path: Option<PathBuf>,
}

// ---- audit --------------------------------------------------------------

#[derive(Debug, Subcommand)]
enum AuditCmd {
    /// List operator-side audit entries (calls `OhdcService.AuditQuery`).
    List {
        /// Lower-bound ISO 8601 timestamp (or `YYYY-MM-DD`).
        #[arg(long)]
        from: Option<String>,
        /// Upper-bound ISO 8601 timestamp (or `YYYY-MM-DD`).
        #[arg(long)]
        to: Option<String>,
        /// Filter to a responder label (best-effort substring match against
        /// the wire entry's `query_params_json` until a typed responder
        /// field lands in the proto).
        #[arg(long)]
        responder: Option<String>,
    },

    /// Export audit entries as CSV for legal / regulatory review.
    Export {
        /// Output CSV path.
        #[arg(long, short)]
        output: PathBuf,
        #[arg(long)]
        from: Option<String>,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        responder: Option<String>,
    },
}

// ---- case-export --------------------------------------------------------

#[derive(Debug, Args)]
struct CaseExportArgs {
    /// The OHD case ULID (26 Crockford-base32 chars).
    #[arg(long)]
    case_ulid: String,

    /// Output JSON path.
    #[arg(long, short)]
    output: PathBuf,
}

// ---- main ---------------------------------------------------------------

fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let kms = kms::KmsBackend::from_str_or_auto(&cli.kms_backend, &kms::CONFIG)?;

    // `login`, `logout`, `cert *`, and `roster *` don't need a network
    // round-trip — handle them outside the tokio runtime.
    match &cli.command {
        Command::Login(args) => return run_login(args, &kms),
        Command::Logout => return run_logout(&kms),
        Command::Cert(c) => {
            let cfg = Config::load(&kms)?;
            return match c {
                CertCmd::Info => cert::cmd_info(cfg.as_ref()),
                CertCmd::Refresh => cert::cmd_refresh(),
                CertCmd::Rotate => cert::cmd_rotate(),
            };
        }
        Command::Roster(r) => {
            let cfg = Config::load(&kms)?;
            return match r {
                RosterCmd::List(p) => {
                    let path = roster::resolve_path(p.roster_path.as_deref(), cfg.as_ref())?;
                    roster::cmd_list(&path)
                }
                RosterCmd::Add { path, label, role } => {
                    let p = roster::resolve_path(path.roster_path.as_deref(), cfg.as_ref())?;
                    roster::cmd_add(&p, label, role)
                }
                RosterCmd::Remove { path, label } => {
                    let p = roster::resolve_path(path.roster_path.as_deref(), cfg.as_ref())?;
                    roster::cmd_remove(&p, label)
                }
                RosterCmd::Status(p) => {
                    let path = roster::resolve_path(p.roster_path.as_deref(), cfg.as_ref())?;
                    roster::cmd_status(&path)
                }
            };
        }
        Command::OidcLogin(_) | Command::Audit(_) | Command::CaseExport(_) => {}
    }

    // Network-bound (and oidc-login) subcommands run on the tokio runtime.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    runtime.block_on(async move {
        match cli.command {
            Command::OidcLogin(args) => run_oidc_login(args, &kms).await,
            Command::Audit(AuditCmd::List {
                from,
                to,
                responder,
            }) => {
                let (storage_url, token) =
                    Config::resolve_storage(cli.storage.as_deref(), cli.token.as_deref(), &kms)?;
                let client =
                    OhdcClient::connect(&storage_url, &token, cli.insecure_skip_verify).await?;
                audit::cmd_list(
                    &client,
                    audit::ListArgs {
                        from: from.as_deref(),
                        to: to.as_deref(),
                        responder: responder.as_deref(),
                    },
                )
                .await
            }
            Command::Audit(AuditCmd::Export {
                output,
                from,
                to,
                responder,
            }) => {
                let (storage_url, token) =
                    Config::resolve_storage(cli.storage.as_deref(), cli.token.as_deref(), &kms)?;
                let client =
                    OhdcClient::connect(&storage_url, &token, cli.insecure_skip_verify).await?;
                audit::cmd_export(
                    &client,
                    &output,
                    audit::ListArgs {
                        from: from.as_deref(),
                        to: to.as_deref(),
                        responder: responder.as_deref(),
                    },
                )
                .await
            }
            Command::CaseExport(args) => {
                let (storage_url, token) =
                    Config::resolve_storage(cli.storage.as_deref(), cli.token.as_deref(), &kms)?;
                let cfg = Config::load(&kms)?;
                let client =
                    OhdcClient::connect(&storage_url, &token, cli.insecure_skip_verify).await?;
                case_export::cmd_case_export(&client, cfg.as_ref(), &args.case_ulid, &args.output)
                    .await
            }
            Command::Login(_) | Command::Logout | Command::Cert(_) | Command::Roster(_) => {
                unreachable!()
            }
        }
    })
}

fn run_login(args: &LoginArgs, kms: &kms::KmsBackend) -> Result<()> {
    // Sanity-check the storage URL parses.
    let _: http::Uri = args
        .storage
        .parse()
        .with_context(|| format!("invalid storage URL: {}", args.storage))?;

    // Preserve any existing OIDC fields when the user re-runs `login` to
    // rotate the storage URL or token (mirrors connect/cli's behaviour).
    let existing = Config::load(kms)?.unwrap_or_default();
    let cfg = Config {
        storage_url: args.storage.clone(),
        token: args.token.clone(),
        refresh_token: existing.refresh_token,
        access_expires_at_ms: existing.access_expires_at_ms,
        oidc_issuer: existing.oidc_issuer,
        oidc_client_id: existing.oidc_client_id,
        oidc_subject: existing.oidc_subject,
        station_label: args.station_label.clone().or(existing.station_label),
        authority_cert: args.authority_cert.clone().or(existing.authority_cert),
        roster_path: args.roster_path.clone().or(existing.roster_path),
    };
    let path = cfg.save(kms)?;
    println!(
        "saved config to {} (kms={}, mode 0600)",
        path.display(),
        kms.name()
    );
    println!("storage:      {}", cfg.storage_url);
    if let Some(s) = &cfg.station_label {
        println!("station:      {s}");
    }
    if let Some(c) = &cfg.authority_cert {
        println!("authority:    {}", c.display());
    }
    Ok(())
}

fn run_logout(kms: &kms::KmsBackend) -> Result<()> {
    let mut existing = match Config::load(kms)? {
        Some(c) => c,
        None => {
            println!("no config found — nothing to do");
            return Ok(());
        }
    };
    existing.token = String::new();
    existing.refresh_token = None;
    existing.access_expires_at_ms = None;
    existing.oidc_subject = None;
    let path = existing.save(kms)?;
    println!(
        "tokens cleared from {} (kms={}); storage URL + station preserved",
        path.display(),
        kms.name()
    );
    Ok(())
}

async fn run_oidc_login(args: OidcLoginArgs, kms: &kms::KmsBackend) -> Result<()> {
    println!("discovering OIDC issuer: {}", args.issuer);
    let device_client =
        oidc::DeviceFlowClient::new(&args.issuer, &args.client_id, None, &args.scope).await?;
    println!("  token_endpoint:  {}", device_client.discovery.token_endpoint);
    println!(
        "  device_endpoint: {}",
        device_client.discovery.device_authorization_endpoint
    );

    let tokens = device_client
        .run(|resp: &oauth2::StandardDeviceAuthorizationResponse| {
            let user_code: &str = resp.user_code().secret();
            println!();
            println!(
                "  Open this URL in any browser: {}",
                resp.verification_uri().as_str()
            );
            if let Some(complete) = resp.verification_uri_complete() {
                let secret: &str = complete.secret();
                println!("  (or this one to skip code entry: {secret})");
            }
            println!("  Enter user code:              {user_code}");
            println!(
                "  Code expires in:              {}s",
                resp.expires_in().as_secs()
            );
            println!();
            println!("Waiting for confirmation… (Ctrl-C to abort)");
        })
        .await?;

    // Merge with any existing config (preserve storage URL the user
    // set up earlier with `login`).
    let existing = Config::load(kms)?.unwrap_or_default();
    let storage_url = args
        .storage
        .clone()
        .or_else(|| {
            if !existing.storage_url.is_empty() {
                Some(existing.storage_url.clone())
            } else {
                None
            }
        })
        .unwrap_or_else(|| args.issuer.clone());

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let access_expires_at_ms = tokens.expires_in_secs.map(|s| now_ms + s * 1000);

    let cfg = Config {
        storage_url,
        token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        access_expires_at_ms,
        oidc_issuer: tokens.oidc_issuer,
        oidc_client_id: Some(args.client_id),
        oidc_subject: tokens.oidc_subject,
        station_label: existing.station_label,
        authority_cert: existing.authority_cert,
        roster_path: existing.roster_path,
    };
    let path = cfg.save(kms)?;
    println!();
    println!(
        "saved config to {} (kms={})",
        path.display(),
        kms.name()
    );
    println!("storage:        {}", cfg.storage_url);
    if let Some(iss) = &cfg.oidc_issuer {
        println!("oidc_issuer:    {iss}");
    }
    println!(
        "access_token:   <set, expires_in={}s>",
        tokens.expires_in_secs.unwrap_or(0)
    );
    if cfg.refresh_token.is_some() {
        println!("refresh_token:  <set>");
    }
    Ok(())
}

fn init_tracing() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .try_init();
}
