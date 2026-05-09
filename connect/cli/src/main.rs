//! OHD Connect CLI — `ohd-connect` binary.
//!
//! This is the v1 implementation of the personal-side CLI. It speaks
//! **OHDC over Connect-RPC** (binary Protobuf, Connect-Protocol-Version
//! headers, gRPC framing) directly against a running OHD Storage server, in
//! a self-session profile per `connect/spec/auth.md`.
//!
//! ## Subcommand surface (v1)
//!
//! | Command | OHDC RPC | Notes |
//! |---|---|---|
//! | `login --storage URL --token TOKEN` | (none) | Writes credentials.toml. Device-flow `/authorize` not yet exposed by storage; tokens are issued out-of-band by `ohd-storage-server issue-self-token --db ...`. |
//! | `whoami` | `Auth.WhoAmI` | Prints `user_ulid` / `token_kind`. |
//! | `health` | `Diag.Health` | Unauthenticated; useful during deploys. |
//! | `log glucose <V> [--unit]` | `Events.PutEvents` | Maps to `std.blood_glucose`, channel `value`. |
//! | `log heart_rate <V>` | `Events.PutEvents` | Maps to `std.heart_rate_resting`. |
//! | `log temperature <V> [--unit]` | `Events.PutEvents` | Maps to `std.body_temperature`. |
//! | `log medication_taken <NAME> [--dose --dose-unit --status]` | `Events.PutEvents` | Maps to `std.medication_dose`. |
//! | `log symptom <NAME> [--severity --location]` | `Events.PutEvents` | Maps to `std.symptom`. |
//! | `query <KIND> [--last-day --last-week --last-month --from --to]` | `Events.QueryEvents` | Server-streaming; renders a small table. |
//! | `version` | (none) | Prints CLI + protocol version (already present in v0). |
//!
//! Every other subcommand listed in `../SPEC.md` "CLI command surface"
//! (`grant`, `pending`, `case`, `audit`, `emergency`, `export`, `config`) is
//! still TBD — those storage RPCs return `Unimplemented` today per
//! `../../storage/STATUS.md`.

use anyhow::{anyhow, Context, Result};
use buffa::MessageField;
use chrono::Utc;
use clap::{Args, Parser, Subcommand};
use futures::StreamExt;

mod client;
mod credentials;
mod events;
mod kms;
mod timeparse;

// OAuth2 device-flow + Crockford-base32 ULID helpers live in the shared
// `ohd-cli-auth` crate (workspace path: storage/crates/ohd-cli-auth).
// Aliased so the rest of the file can keep saying `oidc::` / `ohd_ulid::`.
use ohd_cli_auth::oidc;
use ohd_cli_auth::ulid as ohd_ulid;

/// Generated Connect-RPC client stubs. Produced at build time by
/// `connectrpc-build` from `../../storage/proto/ohdc/v0/ohdc.proto`. The
/// macro expands to a nested `pub mod ohdc { pub mod v0 { … } }` tree
/// holding the buffa message types (Owned + View) and the
/// `OhdcServiceClient<T>` we use for every command but `login` / `version`.
pub mod proto {
    connectrpc::include_generated!();
}

use crate::client::OhdcClient;
use crate::credentials::Credentials;
use crate::events::{build_event_input, query_event_type_alias, render_channel_value, LogKind};
use crate::timeparse::{build_range, render_ms, LastWindow};

/// OHD Connect — personal-side reference application of OHD.
#[derive(Debug, Parser)]
#[command(name = "ohd-connect", version, about, long_about = None)]
struct Cli {
    /// Override the credentials-file storage URL. Useful when targeting a
    /// throwaway local server during development.
    #[arg(long, global = true)]
    storage: Option<String>,

    /// Override the credentials-file token. Useful for one-off calls.
    #[arg(long, global = true)]
    token: Option<String>,

    /// Skip TLS server-cert verification when speaking HTTP/3 over a
    /// `https+h3://` URL. Dev-only; required when the server uses
    /// `http3::dev_self_signed_cert` (no real CA chain). Production HTTP/3
    /// deployments should use a public-CA-issued cert and omit this flag.
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
    /// Print CLI version + protocol version.
    Version,

    /// Save (storage_url, token) to ~/.config/ohd-connect/credentials.toml.
    Login(LoginArgs),

    /// Run OAuth 2.0 Device Authorization Grant against the storage AS.
    OidcLogin(OidcLoginArgs),

    /// Drop tokens from the credentials vault (keeps storage URL).
    Logout,

    /// Call OhdcService.WhoAmI and print the resolved actor.
    Whoami,

    /// Call OhdcService.Health (unauthenticated).
    Health,

    /// Log an event. See module docs above for the per-kind channel mapping.
    #[command(subcommand)]
    Log(LogCmd),

    /// Read events filtered by type and time range.
    Query(QueryArgs),
}

// ---- login ----------------------------------------------------------------

#[derive(Debug, Args)]
struct LoginArgs {
    /// Storage URL, e.g. `http://localhost:8443`. v1 supports plaintext h2c
    /// only; TLS is the deployment's job (Caddy, per
    /// ../../storage/STATUS.md).
    #[arg(long)]
    storage: String,

    /// Self-session bearer token (`ohds_<base32>`). Issue out-of-band with
    /// `ohd-storage-server issue-self-token --db <path>`.
    #[arg(long)]
    token: String,
}

// ---- oidc-login -----------------------------------------------------------

#[derive(Debug, Args)]
struct OidcLoginArgs {
    /// OIDC / OAuth issuer URL. Discovered via .well-known. Typical
    /// values: the storage URL itself (when storage acts as AS),
    /// `https://accounts.google.com`, `https://login.microsoftonline.com/<tenant>/v2.0`.
    #[arg(long)]
    issuer: String,

    /// OAuth client_id registered with the issuer for this CLI.
    #[arg(long, default_value = "ohd-connect-cli")]
    client_id: String,

    /// OAuth/OIDC scopes (space-separated).
    #[arg(long, default_value = "openid offline_access")]
    scope: String,

    /// Storage URL to record alongside the tokens. Defaults to the
    /// issuer URL (storage-as-AS case).
    #[arg(long)]
    storage: Option<String>,
}

// ---- log ------------------------------------------------------------------

#[derive(Debug, Subcommand)]
enum LogCmd {
    /// Log a `std.blood_glucose` event.
    Glucose {
        /// Numeric value. Default unit is mmol/L; pass `--unit mg/dL` for
        /// US-conventional reading (auto-converted to mmol/L on the wire).
        value: f64,
        #[arg(long)]
        unit: Option<String>,
    },

    /// Log a `std.heart_rate_resting` event (single-shot bpm).
    HeartRate {
        /// Beats per minute.
        bpm: f64,
    },

    /// Log a `std.body_temperature` event.
    Temperature {
        /// Numeric value. Default unit is Celsius; pass `--unit F` for
        /// Fahrenheit (auto-converted on the wire).
        value: f64,
        #[arg(long)]
        unit: Option<String>,
    },

    /// Log a `std.medication_dose` event with `status=taken` by default.
    MedicationTaken {
        /// Medication name (free text).
        name: String,
        #[arg(long)]
        dose: Option<f64>,
        #[arg(long, value_name = "UNIT")]
        dose_unit: Option<String>,
        /// Status enum: taken (default) | skipped | late | refused.
        #[arg(long)]
        status: Option<String>,
    },

    /// Log a `std.symptom` event.
    Symptom {
        /// Symptom name (free text).
        name: String,
        /// Integer severity (free-form scale, project default 0–10).
        #[arg(long)]
        severity: Option<i64>,
        #[arg(long)]
        location: Option<String>,
    },
}

impl LogCmd {
    fn into_kind(self) -> LogKind {
        match self {
            LogCmd::Glucose { value, unit } => LogKind::Glucose { value, unit },
            LogCmd::HeartRate { bpm } => LogKind::HeartRate { bpm },
            LogCmd::Temperature { value, unit } => LogKind::Temperature { value, unit },
            LogCmd::MedicationTaken {
                name,
                dose,
                dose_unit,
                status,
            } => LogKind::MedicationTaken {
                name,
                dose,
                dose_unit,
                status,
            },
            LogCmd::Symptom {
                name,
                severity,
                location,
            } => LogKind::Symptom {
                name,
                severity,
                location,
            },
        }
    }
}

// ---- query ----------------------------------------------------------------

#[derive(Debug, Args)]
struct QueryArgs {
    /// Event-type shorthand or fully-qualified name. Recognised aliases:
    /// `glucose`, `heart_rate`, `temperature`, `medication_taken`, `symptom`.
    /// Anything else is passed through verbatim (must be a registered
    /// `<namespace>.<name>` pair).
    kind: String,

    #[arg(long, conflicts_with_all = ["last_week", "last_month", "from", "to"])]
    last_day: bool,
    #[arg(long, conflicts_with_all = ["last_day", "last_month", "from", "to"])]
    last_week: bool,
    #[arg(long, conflicts_with_all = ["last_day", "last_week", "from", "to"])]
    last_month: bool,

    /// ISO8601 lower bound, inclusive (e.g. `2026-05-01` or `2026-05-01T00:00:00Z`).
    #[arg(long)]
    from: Option<String>,
    /// ISO8601 upper bound, exclusive.
    #[arg(long)]
    to: Option<String>,

    /// Cap number of returned events (default 100).
    #[arg(long, default_value_t = 100_i64)]
    limit: i64,
}

// ---- main -----------------------------------------------------------------

fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let kms = kms::KmsBackend::from_str_or_auto(&cli.kms_backend, &kms::CONFIG)?;

    // `version`, `login`, and `logout` don't need a network round trip —
    // handle them outside the tokio runtime to keep startup fast.
    match cli.command {
        Command::Version => {
            println!("ohd-connect {}", env!("CARGO_PKG_VERSION"));
            println!("ohdc protocol: v1");
            return Ok(());
        }
        Command::Login(args) => {
            return run_login(args, &kms);
        }
        Command::Logout => {
            return run_logout(&kms);
        }
        _ => {}
    }

    // The remaining commands are async. Build a minimal multi-thread runtime;
    // the connectrpc HTTP/2 client needs a tokio reactor for hyper.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("build tokio runtime")?;
    runtime.block_on(async move {
        match cli.command {
            Command::OidcLogin(args) => return run_oidc_login(args, &kms).await,
            Command::Whoami | Command::Health | Command::Log(_) | Command::Query(_) => {
                let (storage_url, token) =
                    credentials::resolve(cli.storage.as_deref(), cli.token.as_deref(), &kms)?;
                let client =
                    OhdcClient::connect(&storage_url, &token, cli.insecure_skip_verify)
                        .await?;
                match cli.command {
                    Command::Whoami => run_whoami(&client).await,
                    Command::Health => run_health(&client).await,
                    Command::Log(log) => run_log(&client, log.into_kind()).await,
                    Command::Query(args) => run_query(&client, args).await,
                    _ => unreachable!(),
                }
            }
            Command::Version | Command::Login(_) | Command::Logout => unreachable!(),
        }
    })
}

// ---- command implementations ---------------------------------------------

fn run_login(args: LoginArgs, kms: &kms::KmsBackend) -> Result<()> {
    let creds = Credentials {
        storage_url: args.storage,
        token: args.token,
        ..Default::default()
    };
    // Sanity-check that the storage URL parses. (We don't actually connect
    // here — that gates `whoami` in the demo script.)
    let _: http::Uri = creds
        .storage_url
        .parse()
        .with_context(|| format!("invalid storage URL: {}", creds.storage_url))?;

    let path = creds.save(kms)?;
    println!(
        "saved credentials to {} (kms={}, mode 0600)",
        path.display(),
        kms.name()
    );
    println!("storage: {}", creds.storage_url);
    Ok(())
}

fn run_logout(kms: &kms::KmsBackend) -> Result<()> {
    let mut existing = match Credentials::load(kms)? {
        Some(c) => c,
        None => {
            println!("no credentials found — nothing to do");
            return Ok(());
        }
    };
    existing.token = String::new();
    existing.refresh_token = None;
    existing.access_expires_at_ms = None;
    existing.oidc_subject = None;
    let path = existing.save(kms)?;
    println!(
        "tokens cleared from {} (kms={}); storage URL preserved",
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
            // Unfortunately oauth2 5.x's StandardDeviceAuthorizationResponse
            // has the URL/code as private fields; use accessors.
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

    // Merge with any existing credentials (preserve storage URL the user
    // set up earlier with `login`).
    let existing = Credentials::load(kms)?.unwrap_or_default();
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

    let creds = Credentials {
        storage_url,
        token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        access_expires_at_ms,
        oidc_issuer: tokens.oidc_issuer,
        oidc_client_id: Some(args.client_id),
        oidc_subject: tokens.oidc_subject,
    };
    let path = creds.save(kms)?;
    println!();
    println!(
        "saved credentials to {} (kms={})",
        path.display(),
        kms.name()
    );
    println!("storage:        {}", creds.storage_url);
    if let Some(iss) = &creds.oidc_issuer {
        println!("oidc_issuer:    {iss}");
    }
    println!(
        "access_token:   <set, expires_in={}s>",
        tokens.expires_in_secs.unwrap_or(0)
    );
    if creds.refresh_token.is_some() {
        println!("refresh_token:  <set>");
    }
    Ok(())
}

async fn run_whoami(client: &OhdcClient) -> Result<()> {
    use crate::proto::ohdc::v0 as pb;
    let owned = client.who_am_i(pb::WhoAmIRequest::default()).await?;
    let user_ulid_s = match owned.user_ulid.as_option() {
        Some(u) => ohd_ulid::render_ulid_bytes(&u.bytes),
        None => "<unset>".to_string(),
    };
    println!("storage:    {}", client.storage_url);
    println!("user_ulid:  {user_ulid_s}");
    println!("token_kind: {}", owned.token_kind);
    if let Some(label) = owned.grantee_label.as_ref() {
        if !label.is_empty() {
            println!("grantee:    {label}");
        }
    }
    Ok(())
}

async fn run_health(client: &OhdcClient) -> Result<()> {
    use crate::proto::ohdc::v0 as pb;
    let owned = client.health(pb::HealthRequest::default()).await?;
    println!("status:           {}", owned.status);
    println!("server_version:   {}", owned.server_version);
    println!("protocol_version: {}", owned.protocol_version);
    println!("server_time_ms:   {}", owned.server_time_ms);
    Ok(())
}

async fn run_log(client: &OhdcClient, kind: LogKind) -> Result<()> {
    use crate::proto::ohdc::v0 as pb;
    let now_ms = Utc::now().timestamp_millis();
    let event = build_event_input(&kind, now_ms)?;
    let req = pb::PutEventsRequest {
        events: vec![event],
        atomic: false,
        ..Default::default()
    };
    let owned = client.put_events(req).await?;
    if owned.results.is_empty() {
        return Err(anyhow!("PutEvents returned no results"));
    }
    let mut had_error = false;
    for r in owned.results {
        match r.outcome {
            Some(pb::put_event_result::Outcome::Committed(c)) => {
                let ulid = c
                    .ulid
                    .as_option()
                    .map(|u| ohd_ulid::render_ulid_bytes(&u.bytes))
                    .unwrap_or_else(|| "<unset>".to_string());
                println!("committed {ulid} at {} ms", c.committed_at_ms);
            }
            Some(pb::put_event_result::Outcome::Pending(p)) => {
                let ulid = p
                    .ulid
                    .as_option()
                    .map(|u| ohd_ulid::render_ulid_bytes(&u.bytes))
                    .unwrap_or_else(|| "<unset>".to_string());
                println!(
                    "pending   {ulid} (expires_at_ms = {})",
                    p.expires_at_ms
                );
            }
            Some(pb::put_event_result::Outcome::Error(e)) => {
                eprintln!("error    {} — {}", e.code, e.message);
                had_error = true;
            }
            None => {
                eprintln!("error    <empty outcome>");
                had_error = true;
            }
        }
    }
    if had_error {
        Err(anyhow!("one or more events failed"))
    } else {
        Ok(())
    }
}

async fn run_query(client: &OhdcClient, args: QueryArgs) -> Result<()> {
    use crate::proto::ohdc::v0 as pb;

    // Resolve `--last-*` / `--from` / `--to`.
    let last = match (args.last_day, args.last_week, args.last_month) {
        (true, false, false) => Some(LastWindow::Day),
        (false, true, false) => Some(LastWindow::Week),
        (false, false, true) => Some(LastWindow::Month),
        (false, false, false) => None,
        _ => return Err(anyhow!("--last-day, --last-week, --last-month are mutually exclusive")),
    };
    let range = build_range(last, args.from.as_deref(), args.to.as_deref())?;

    // Resolve the type alias. Recognised short forms map to canonical names;
    // fully-qualified `<ns>.<name>` is passed through verbatim.
    let event_type = if args.kind.contains('.') {
        args.kind.clone()
    } else {
        query_event_type_alias(&args.kind)
            .ok_or_else(|| {
                anyhow!(
                    "unknown event-type short form {:?}; pass a fully-qualified \
                     `<namespace>.<name>` (e.g. `std.blood_glucose`) or one of the \
                     recognized aliases: glucose, heart_rate, temperature, \
                     medication_taken, symptom",
                    args.kind
                )
            })?
            .to_string()
    };

    let filter = pb::EventFilter {
        from_ms: range.from_ms,
        to_ms: range.to_ms,
        event_types_in: vec![event_type.clone()],
        include_superseded: true,
        limit: Some(args.limit),
        ..Default::default()
    };

    let mut stream = client
        .query_events(pb::QueryEventsRequest {
            filter: MessageField::some(filter),
            ..Default::default()
        })
        .await?;

    println!(
        "{:<26}  {:<25}  {:<24}  {}",
        "ULID", "TIMESTAMP (UTC)", "TYPE", "CHANNELS"
    );
    let mut count = 0_u64;
    while let Some(item) = stream.next().await {
        let event = item?;
        let ulid_s = match event.ulid.as_option() {
            Some(u) => ohd_ulid::render_ulid_bytes(&u.bytes),
            None => "<no-ulid>".to_string(),
        };
        let ts_s = render_ms(event.timestamp_ms);
        let chan_s = event
            .channels
            .iter()
            .map(render_channel_value)
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{:<26}  {:<25}  {:<24}  {}",
            ulid_s, ts_s, event.event_type, chan_s
        );
        count += 1;
    }
    eprintln!("({count} event{})", if count == 1 { "" } else { "s" });
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
