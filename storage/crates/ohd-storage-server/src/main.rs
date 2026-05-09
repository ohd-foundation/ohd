//! OHD Storage server binary.
//!
//! Wire transport: Connect-RPC over HTTP/1.1 + HTTP/2 (auto-negotiated by
//! hyper) **and** HTTP/3 (quinn + h3) when `--http3-listen` is set. Both
//! listeners share one [`server::connect_service`] so handler bodies are
//! identical regardless of transport. See STATUS.md "HTTP/3 (in-binary)
//! — landed" for details.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use ohd_storage_core::{
    audit, auth as ohd_auth,
    grants::{self as ohd_grants, NewGrant, RuleEffect},
    ohdc as ohd_ohdc,
    storage::{Storage, StorageConfig},
    PROTOCOL_VERSION, STORAGE_VERSION,
};
use rusqlite::params;

mod auth_server;
mod http3;
mod jwks;
mod oauth;
mod relay_client;
mod server;
mod sync_server;

/// Generated Connect-RPC service stubs. Produced at build time by
/// `connectrpc-build` from `proto/ohdc/v0/ohdc.proto` (see `build.rs`).
///
/// The macro expands to a nested `pub mod ohdc { pub mod v0 { … } }` tree
/// holding the buffa message types (Owned + View), the `OhdcService` trait
/// + `OhdcServiceExt::register`, and the `OhdcServiceClient<T>` for the
/// client-side test harness.
pub mod proto {
    connectrpc::include_generated!();
}

/// OHD Storage server — Connect-RPC over HTTP/2 (HTTP/3 next).
#[derive(Debug, Parser)]
#[command(
    name = "ohd-storage-server",
    version,
    about = "OHD Storage server (OHDC over Connect-RPC)",
    long_about = "OHD Storage hosts the per-user health-data files and exposes the OHDC \
                  protocol (Connect-RPC over HTTP/1.1 + HTTP/2; HTTP/3 next). One \
                  external surface, three auth profiles (self-session / grant / device)."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print a self-test health summary and exit.
    Health,
    /// Initialize a new storage file at the given path. Stamps `_meta.user_ulid`
    /// and seeds the standard registry.
    Init {
        /// Path to the per-user `data.db` file.
        #[arg(long)]
        db: PathBuf,
        /// Optional cipher key (hex; 64 chars = 32 bytes). Empty = unencrypted.
        #[arg(long)]
        cipher_key: Option<String>,
    },
    /// Issue a self-session token against an existing file.
    IssueSelfToken {
        /// Path to `data.db`.
        #[arg(long)]
        db: PathBuf,
        /// Optional cipher key (hex).
        #[arg(long)]
        cipher_key: Option<String>,
        /// Display label for the issued token.
        #[arg(long, default_value = "cli")]
        label: String,
    },
    /// Serve OHDC over the network on the given address.
    Serve {
        /// Path to `data.db`.
        #[arg(long)]
        db: PathBuf,
        /// Listen address. Default `0.0.0.0:8443` (HTTP/3-style port; v1
        /// serves HTTP/2 over plaintext on it. Front it with Caddy or
        /// equivalent for TLS termination.)
        #[arg(long, default_value = "0.0.0.0:8443")]
        listen: SocketAddr,
        /// Backwards-compatible alias for `--listen`. The brief calls out
        /// `--port`; honoured here as a port-only override.
        #[arg(long)]
        port: Option<u16>,
        /// Optional cipher key (hex).
        #[arg(long)]
        cipher_key: Option<String>,
        /// Disable CORS entirely. Default: a permissive CORS layer is on so a
        /// browser dev server (Care web at `http://localhost:5173`) can reach
        /// `http://localhost:18443` directly. Production deployments should
        /// front the storage with Caddy and disable this with `--no-cors`.
        #[arg(long, default_value_t = false)]
        no_cors: bool,
        /// Optional UDP/QUIC listen address for HTTP/3. When set, the server
        /// runs an in-process HTTP/3 (Connect-RPC) listener alongside the
        /// HTTP/1.1 + HTTP/2 listener on `--listen`. Self-signed cert is
        /// generated for `localhost` / `127.0.0.1` in dev unless
        /// `--http3-cert` / `--http3-key` are supplied.
        #[arg(long)]
        http3_listen: Option<SocketAddr>,
        /// PEM-encoded certificate chain for the HTTP/3 listener (one or
        /// more `CERTIFICATE` blocks; matches Let's Encrypt `fullchain.pem`).
        /// Must be paired with `--http3-key`. When omitted, a self-signed
        /// dev cert is generated and a warning is printed to stderr.
        #[arg(long, value_name = "PATH", requires = "http3_key")]
        http3_cert: Option<PathBuf>,
        /// PEM-encoded private key for the HTTP/3 listener (PKCS#8, PKCS#1,
        /// or SEC1). Must be paired with `--http3-cert`.
        #[arg(long, value_name = "PATH", requires = "http3_cert")]
        http3_key: Option<PathBuf>,
        /// Outbound relay tunnel: `host:port` of the relay's
        /// `--quic-tunnel-listen` endpoint (ALPN `ohd-tnl1`, default port
        /// 9001). When set, storage opens a raw-QUIC tunnel to the relay
        /// alongside the HTTP/2 + HTTP/3 listeners; the relay then routes
        /// consumer attaches over that tunnel into the same Connect-RPC
        /// service. Migration-resilient across mobile network handoffs.
        #[arg(long)]
        relay_url: Option<String>,
        /// Long-lived credential issued at `POST /v1/register` time (ASCII
        /// base32). Required when `--relay-url` is set.
        #[arg(long, requires = "relay_url", env = "OHD_RELAY_CREDENTIAL")]
        relay_credential: Option<String>,
        /// Rendezvous-id (registration token) issued at `POST /v1/register`
        /// time. Required when `--relay-url` is set.
        #[arg(long, requires = "relay_url", env = "OHD_RELAY_REGISTRATION_TOKEN")]
        relay_registration_token: Option<String>,
        /// Optional SHA-256 of the relay's QUIC tunnel certificate (DER),
        /// hex-encoded. When supplied, the TLS handshake against the relay
        /// only succeeds if the leaf cert hashes to this value, sidestepping
        /// the WebPKI / OS trust store.
        #[arg(long, requires = "relay_url")]
        relay_cert_pin: Option<String>,
        /// Dev only: accept any cert from the relay. Use with self-signed
        /// dev relays.
        #[arg(long, default_value_t = false, requires = "relay_url")]
        relay_allow_insecure: bool,
        /// Optional OIDC issuer URL. When set, the server lights up
        /// `/.well-known/openid-configuration` + `/oauth/*` endpoints making
        /// the storage instance act as its own OAuth 2.0 + OIDC IdP. The URL
        /// is what consumers see in id_tokens' `iss` claim and in the
        /// discovery doc — it must match the deployment's externally-visible
        /// origin (e.g. `https://storage.example.com`). Default off — most
        /// deployments delegate to external IdPs.
        #[arg(long, value_name = "URL")]
        oauth_issuer: Option<String>,
    },
    /// Issue a grant token bound to a freshly-created grant row. The grant
    /// is owned by the file's `_meta.user_ulid`. Demos / tactical use only;
    /// the canonical flow is `OhdcService.CreateGrant` from a self-session
    /// (currently stubbed — see STATUS.md).
    IssueGrantToken {
        /// Path to `data.db`.
        #[arg(long)]
        db: PathBuf,
        /// CSV of event types this grant can read (e.g.
        /// `std.blood_glucose,std.heart_rate_resting`). Empty = none.
        #[arg(long, default_value = "")]
        read: String,
        /// CSV of event types this grant can write. Empty = none.
        #[arg(long, default_value = "")]
        write: String,
        /// Approval mode: `always` | `auto_for_event_types` | `never_required`.
        #[arg(long, default_value = "always")]
        approval_mode: String,
        /// Display label for the grantee (shown in audit + WhoAmI).
        #[arg(long, default_value = "Dr. Smith")]
        label: String,
        /// Free-text purpose for the grant.
        #[arg(long)]
        purpose: Option<String>,
        /// Days until expiry. Default 30.
        #[arg(long, default_value_t = 30_i64)]
        expires_days: i64,
        /// Optional cipher key (hex).
        #[arg(long)]
        cipher_key: Option<String>,
    },
    /// Tactical: list pending events directly from SQLite (no RPC; the
    /// `OhdcService.ListPending` handler is stubbed). For demo + dev only.
    PendingList {
        /// Path to `data.db`.
        #[arg(long)]
        db: PathBuf,
        /// Optional cipher key (hex).
        #[arg(long)]
        cipher_key: Option<String>,
    },
    /// Tactical: approve a pending event by ULID. Promotes the row from
    /// `pending_events` into `events` + `event_channels`, preserving the ULID.
    /// Operates directly on SQLite. For demo + dev only.
    PendingApprove {
        /// Path to `data.db`.
        #[arg(long)]
        db: PathBuf,
        /// Crockford-base32 ULID of the pending row.
        #[arg(long)]
        ulid: String,
        /// Optional cipher key (hex).
        #[arg(long)]
        cipher_key: Option<String>,
    },
}

fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    match cli.command {
        Command::Health => {
            let summary = ohd_ohdc::health()?;
            println!(
                "OHD Storage server v{STORAGE_VERSION} \u{2014} health: {} (protocol {PROTOCOL_VERSION})",
                summary.status
            );
            Ok(())
        }
        Command::Init { db, cipher_key } => {
            let key = parse_key(cipher_key.as_deref())?;
            let storage = Storage::open(StorageConfig::new(db).with_cipher_key(key))?;
            println!(
                "initialized storage at {} (user_ulid={})",
                storage.path().display(),
                ohd_storage_core::ulid::to_crockford(&storage.user_ulid())
            );
            Ok(())
        }
        Command::IssueSelfToken {
            db,
            cipher_key,
            label,
        } => {
            let key = parse_key(cipher_key.as_deref())?;
            let storage = Storage::open(StorageConfig::new(db).with_cipher_key(key))?;
            let user_ulid = storage.user_ulid();
            let token = storage.with_conn(|conn| {
                ohd_auth::issue_self_session_token(conn, user_ulid, Some(&label), None)
            })?;
            // Audit issuance.
            storage.with_conn(|conn| {
                audit::append(
                    conn,
                    &audit::AuditEntry {
                        ts_ms: audit::now_ms(),
                        actor_type: audit::ActorType::Self_,
                        auto_granted: false,
                        grant_id: None,
                        action: "login".into(),
                        query_kind: Some("issue_self_session_token".into()),
                        query_params_json: None,
                        rows_returned: None,
                        rows_filtered: None,
                        result: audit::AuditResult::Success,
                        reason: None,
                        caller_ip: None,
                        caller_ua: None,
                        delegated_for_user_ulid: None,
                    },
                )
            })?;
            println!("{token}");
            Ok(())
        }
        Command::Serve {
            db,
            listen,
            port,
            cipher_key,
            no_cors,
            http3_listen,
            http3_cert,
            http3_key,
            relay_url,
            relay_credential,
            relay_registration_token,
            relay_cert_pin,
            relay_allow_insecure,
            oauth_issuer,
        } => {
            let addr = match port {
                Some(p) => SocketAddr::new(listen.ip(), p),
                None => listen,
            };
            let key = parse_key(cipher_key.as_deref())?;
            let storage = Arc::new(Storage::open(StorageConfig::new(db).with_cipher_key(key))?);
            // `server::serve` runs the OAuth state-table + signing-key
            // bootstrap when an issuer is configured. Mirror in the relay
            // tunnel path below — the same `Storage` handle is shared so
            // running it twice is a harmless no-op.
            if oauth_issuer.is_some() {
                tracing::info!(issuer = ?oauth_issuer, "OAuth/OIDC IdP endpoints enabled");
            }
            tracing::info!(addr=%addr, h3=?http3_listen, path=%storage.path().display(), cors=!no_cors, oauth=?oauth_issuer, "serving OHDC");
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async move {
                // Always run the HTTP/2 listener; optionally a sibling HTTP/3
                // listener over the same `Storage` handle. Both share one
                // ConnectRpcService (built off the same Router) so handler
                // bodies are identical regardless of transport.
                let h2_storage = Arc::clone(&storage);
                let oauth_issuer_for_h2 = oauth_issuer.clone();
                let h2_task = tokio::spawn(async move {
                    server::serve(h2_storage, addr, !no_cors, oauth_issuer_for_h2).await
                });
                let h3_task = if let Some(h3_addr) = http3_listen {
                    let h3_storage = Arc::clone(&storage);
                    let svc = server::connect_service(h3_storage);
                    // P0 cert resolution: production cert/key pair if both
                    // flags supplied, else generate a dev self-signed cert
                    // and emit a stderr warning. clap's `requires` keyword on
                    // each flag enforces "both or neither" at parse time, so
                    // the asymmetric arms below shouldn't actually fire.
                    let (cert, pkey) = match (http3_cert.as_ref(), http3_key.as_ref()) {
                        (Some(cp), Some(kp)) => http3::load_pem_cert_key(cp, kp)?,
                        (None, None) => {
                            eprintln!(
                                "WARNING: --http3-listen set without --http3-cert/--http3-key; \
                                 using a dev self-signed cert (localhost / 127.0.0.1). \
                                 Production deployments must supply real PEM materials."
                            );
                            http3::dev_self_signed_cert()?
                        }
                        _ => unreachable!("clap `requires` enforces pairing"),
                    };
                    Some(tokio::spawn(async move {
                        http3::serve(h3_addr, svc, cert, pkey).await
                    }))
                } else {
                    None
                };

                // Optional outbound relay tunnel (raw QUIC, ALPN ohd-tnl1).
                // Mirrors the relay's `--quic-tunnel-listen` server side. The
                // tunnel runs alongside the HTTP/2 + HTTP/3 listeners; the
                // relay routes consumer attaches into the same Connect-RPC
                // service. Connection migration + reconnect-with-backoff is
                // wired internally; we only need to spin the task and pass
                // the shutdown signal.
                let (relay_shutdown_tx, relay_shutdown_rx) = tokio::sync::watch::channel(false);
                let relay_task = if let Some(url) = relay_url.clone() {
                    let cred = relay_credential.clone().ok_or_else(|| {
                        anyhow::anyhow!("--relay-url requires --relay-credential")
                    })?;
                    let token = relay_registration_token.clone().ok_or_else(|| {
                        anyhow::anyhow!("--relay-url requires --relay-registration-token")
                    })?;
                    let pin = match relay_cert_pin.as_deref() {
                        None => None,
                        Some(s) => Some(
                            hex::decode(s.trim())
                                .map_err(|e| anyhow::anyhow!("--relay-cert-pin not hex: {e}"))?,
                        ),
                    };
                    let opts = relay_client::RelayClientOptions {
                        relay_url: url,
                        registration_token: token,
                        credential: cred,
                        expected_relay_pubkey_pin: pin,
                        allow_insecure_dev: relay_allow_insecure,
                    };
                    let svc = server::connect_service(Arc::clone(&storage));
                    Some(tokio::spawn(async move {
                        if let Err(err) =
                            relay_client::serve_relay_tunnel(opts, svc, relay_shutdown_rx).await
                        {
                            tracing::warn!(?err, "relay tunnel client exited with error");
                        }
                    }))
                } else {
                    drop(relay_shutdown_rx);
                    None
                };

                let h2_res = h2_task.await.expect("h2 task panicked");
                if let Some(h) = h3_task {
                    // We aborted on h2 finish; ignore the h3 result.
                    h.abort();
                }
                if let Some(h) = relay_task {
                    let _ = relay_shutdown_tx.send(true);
                    h.abort();
                }
                drop(relay_shutdown_tx);
                h2_res
            })?;
            Ok(())
        }
        Command::IssueGrantToken {
            db,
            read,
            write,
            approval_mode,
            label,
            purpose,
            expires_days,
            cipher_key,
        } => run_issue_grant_token(
            db,
            cipher_key.as_deref(),
            &label,
            purpose.as_deref(),
            &approval_mode,
            &read,
            &write,
            expires_days,
        ),
        Command::PendingList { db, cipher_key } => {
            let key = parse_key(cipher_key.as_deref())?;
            let storage = Storage::open(StorageConfig::new(db).with_cipher_key(key))?;
            run_pending_list(&storage)
        }
        Command::PendingApprove {
            db,
            ulid,
            cipher_key,
        } => {
            let key = parse_key(cipher_key.as_deref())?;
            let storage = Storage::open(StorageConfig::new(db).with_cipher_key(key))?;
            run_pending_approve(&storage, &ulid)
        }
    }
}

/// Parse `--read` / `--write` CSV into a vec of (event_type, allow) rule pairs.
/// Empty / whitespace-only entries are dropped silently.
fn parse_csv_rules(csv: &str) -> Vec<(String, RuleEffect)> {
    csv.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| (s.to_string(), RuleEffect::Allow))
        .collect()
}

/// Validate the approval_mode literal matches one of the three values the
/// schema's `grants.approval_mode CHECK` accepts.
fn validate_approval_mode(s: &str) -> anyhow::Result<()> {
    match s {
        "always" | "auto_for_event_types" | "never_required" => Ok(()),
        other => Err(anyhow::anyhow!(
            "invalid --approval-mode {other:?}; expected 'always' | 'auto_for_event_types' | 'never_required'"
        )),
    }
}

fn run_issue_grant_token(
    db: PathBuf,
    cipher_key: Option<&str>,
    label: &str,
    purpose: Option<&str>,
    approval_mode: &str,
    read_csv: &str,
    write_csv: &str,
    expires_days: i64,
) -> anyhow::Result<()> {
    eprintln!(
        "DEPRECATED: `issue-grant-token` is a tactical helper. \
         Prefer the wire RPC `OhdcService.CreateGrant` via ohd-connect; \
         this CLI path will be removed in v1.x."
    );
    validate_approval_mode(approval_mode)?;
    let key = parse_key(cipher_key)?;
    let storage = Storage::open(StorageConfig::new(db).with_cipher_key(key))?;
    let user_ulid = storage.user_ulid();

    // Best-effort registry seeding for the demo: `std.clinical_note` is the
    // doctor-side write target for the OHD Care end-to-end demo, but the
    // canonical migration (`002_std_registry.sql`) hasn't picked it up yet
    // (that's a v1.x deliverable). We INSERT OR IGNORE the type + channels so
    // the grant rule resolution succeeds without touching ohd-storage-core.
    seed_clinical_note_type_if_missing(&storage)?;

    let read_rules = parse_csv_rules(read_csv);
    let write_rules = parse_csv_rules(write_csv);

    // Default action: 'allow' if any read rule is set (allowlist closes the gap
    // via deny-by-omission at the registry level), 'deny' if no rules — the
    // closed-by-default posture from spec/privacy-access.md.
    let default_action = if read_rules.is_empty() {
        RuleEffect::Deny
    } else {
        RuleEffect::Deny
    };

    let now = ohd_storage_core::audit::now_ms();
    let expires_at_ms = if expires_days > 0 {
        Some(now + expires_days * 86_400_000)
    } else {
        None
    };

    let new_grant = NewGrant {
        grantee_label: label.to_string(),
        grantee_kind: "human".to_string(),
        purpose: purpose.map(str::to_string),
        default_action,
        approval_mode: approval_mode.to_string(),
        expires_at_ms,
        event_type_rules: read_rules.clone(),
        channel_rules: vec![],
        sensitivity_rules: vec![],
        write_event_type_rules: write_rules.clone(),
        auto_approve_event_types: vec![],
        aggregation_only: false,
        strip_notes: false,
        notify_on_access: false,
        require_approval_per_query: false,
        max_queries_per_day: None,
        max_queries_per_hour: None,
        rolling_window_days: None,
        absolute_window: None,
        delegate_for_user_ulid: None,
        // Multi-storage delegate channel-encryption key handoff (concurrent
        // closeout agent landed in core). v0 device/grant tokens issued from
        // the CLI helper still target a single storage, so no recovery
        // pubkey is wired through. Future revision: accept a
        // `--grantee-recovery-pubkey` flag.
        grantee_recovery_pubkey: None,
    };

    // create_grant + issue_grant_token cooperate: the row is created via the
    // grants module (transactional), then a bearer token is bound to its rowid
    // via the auth module.
    let (grant_id, _grant_ulid) =
        storage.with_conn_mut(|conn| ohd_grants::create_grant(conn, &new_grant))?;
    let ttl_ms = expires_days.checked_mul(86_400_000);
    let token = storage.with_conn(|conn| {
        ohd_auth::issue_grant_token(
            conn,
            user_ulid,
            grant_id,
            ohd_auth::TokenKind::Grant,
            ttl_ms,
        )
    })?;

    // Audit: a `grant_create` row makes it discoverable via OhdcService.AuditQuery.
    storage.with_conn(|conn| {
        audit::append(
            conn,
            &audit::AuditEntry {
                ts_ms: audit::now_ms(),
                actor_type: audit::ActorType::Self_,
                auto_granted: false,
                grant_id: Some(grant_id),
                action: "grant_create".into(),
                query_kind: Some("issue_grant_token".into()),
                query_params_json: Some(format!(
                    "{{\"label\":{label:?},\"approval_mode\":{approval_mode:?},\
                      \"read\":[{}],\"write\":[{}]}}",
                    read_rules
                        .iter()
                        .map(|(t, _)| format!("{t:?}"))
                        .collect::<Vec<_>>()
                        .join(","),
                    write_rules
                        .iter()
                        .map(|(t, _)| format!("{t:?}"))
                        .collect::<Vec<_>>()
                        .join(","),
                )),
                rows_returned: None,
                rows_filtered: None,
                result: audit::AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )
    })?;

    println!("{token}");
    Ok(())
}

/// Tactical pending-list: dump every row in `pending_events` (any status) with
/// a brief one-line summary. The wire RPC `OhdcService.ListPending` now exists
/// and should be preferred; this helper remains as a deprecated ops shortcut
/// for direct DB inspection (no auth, no audit row).
fn run_pending_list(storage: &Storage) -> anyhow::Result<()> {
    eprintln!(
        "DEPRECATED: `pending-list` is a tactical helper. \
         Prefer the wire RPC `OhdcService.ListPending` via ohd-connect; \
         this CLI path will be removed in v1.x."
    );
    // We collect into a Vec inside the with_conn closure (which expects an
    // ohd_storage_core::Result) so we can release the SQLite mutex before
    // any println!, and so type inference doesn't have to chase the rusqlite
    // crate (we deliberately don't depend on it directly here — it's a
    // transitive of ohd-storage-core).
    type Row = (Vec<u8>, i64, i64, String, i64, String, Option<String>);
    let rows: Vec<Row> = storage.with_conn(|conn| -> ohd_storage_core::Result<Vec<Row>> {
        let mut stmt = conn.prepare(
            "SELECT p.ulid_random, p.submitted_at_ms, p.submitting_grant_id, p.status,
                    p.expires_at_ms, p.payload_json, g.grantee_label
               FROM pending_events p
               LEFT JOIN grants g ON g.id = p.submitting_grant_id
              ORDER BY p.submitted_at_ms DESC",
        )?;
        let mut out = Vec::new();
        let mut iter = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Vec<u8>>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        while let Some(row) = iter.next() {
            out.push(row?);
        }
        Ok(out)
    })?;

    if rows.is_empty() {
        println!("(no pending events)");
        return Ok(());
    }
    println!(
        "{:<26}  {:<8}  {:<24}  {:<24}  {}",
        "ULID", "STATUS", "GRANTEE", "EVENT_TYPE", "SUBMITTED_AT"
    );
    for (rand_tail, submitted, _grant_id, status, _expires, payload, grantee) in rows {
        // Best-effort decode of the event_type + timestamp_ms out of
        // payload_json. The ULID's time prefix is derived from the event's
        // own timestamp_ms (the spec) — _not_ the submission time, which is
        // a separate column on the row. Falls back to submitted_at_ms only
        // when the payload is unparseable (shouldn't happen).
        let parsed: Option<serde_json::Value> = serde_json::from_str(&payload).ok();
        let event_type = parsed
            .as_ref()
            .and_then(|v| {
                v.get("event_type")
                    .and_then(|e| e.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| "?".into());
        let event_ts_ms: i64 = parsed
            .as_ref()
            .and_then(|v| v.get("timestamp_ms").and_then(|t| t.as_i64()))
            .unwrap_or(submitted);
        let ulid = ohd_storage_core::ulid::from_parts(event_ts_ms, &rand_tail).unwrap_or_default();
        let ulid_s = ohd_storage_core::ulid::to_crockford(&ulid);
        println!(
            "{:<26}  {:<8}  {:<24}  {:<24}  {}",
            ulid_s,
            status,
            grantee.as_deref().unwrap_or("?"),
            event_type,
            submitted
        );
    }
    Ok(())
}

/// Tactical pending-approve: promote a `pending_events` row into `events`
/// (preserving its ULID), inserting `event_channels` from `payload_json`. The
/// wire RPC `OhdcService.ApprovePending` now exists and should be preferred;
/// this helper remains as a deprecated ops shortcut for direct DB action.
fn run_pending_approve(storage: &Storage, ulid_str: &str) -> anyhow::Result<()> {
    eprintln!(
        "DEPRECATED: `pending-approve` is a tactical helper. \
         Prefer the wire RPC `OhdcService.ApprovePending` via ohd-connect; \
         this CLI path will be removed in v1.x."
    );
    let ulid = ohd_storage_core::ulid::parse_crockford(ulid_str)
        .map_err(|e| anyhow::anyhow!("parse ULID {ulid_str}: {e}"))?;
    let rand_tail: [u8; 10] = ohd_storage_core::ulid::random_tail(&ulid);

    let event_id = storage.with_conn_mut(|conn| -> ohd_storage_core::Result<i64> {
        let tx = conn.transaction()?;
        // Look the pending row up by its rand_tail (the unique index in the
        // schema). Bail if it's not pending — this preserves the spec invariant
        // that approve is idempotent only when the row is still queued.
        let row: Option<(i64, i64, i64, String, String)> = {
            let mut stmt = tx.prepare(
                "SELECT id, submitted_at_ms, submitting_grant_id, status, payload_json
                   FROM pending_events
                  WHERE ulid_random = ?1",
            )?;
            stmt.query_row(params![rand_tail.to_vec()], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
            })
            .ok()
        };
        let Some((pending_id, _submitted_at_ms, grant_id, status, payload_json)) = row else {
            return Err(ohd_storage_core::Error::NotFound);
        };
        if status != "pending" {
            return Err(ohd_storage_core::Error::InvalidArgument(format!(
                "pending row is in status {status:?}, not 'pending'"
            )));
        }

        // Decode the original payload back into a CoreEventInput.
        let input: ohd_storage_core::events::EventInput = serde_json::from_str(&payload_json)?;

        // Resolve event_type id.
        let etn = ohd_storage_core::registry::EventTypeName::parse(&input.event_type)?;
        let etype = ohd_storage_core::registry::resolve_event_type(&tx, &etn)?;

        // Insert into `events` reusing the pending ULID.
        tx.execute(
            "INSERT INTO events
                (ulid_random, timestamp_ms, tz_offset_minutes, tz_name, duration_ms,
                 event_type_id, source, source_id, notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                rand_tail.to_vec(),
                input.timestamp_ms,
                input.tz_offset_minutes,
                input.tz_name,
                input.duration_ms,
                etype.id,
                input.source,
                input.source_id,
                input.notes,
            ],
        )?;
        let event_id = tx.last_insert_rowid();

        // Insert each channel value. We re-resolve the channel id per row.
        for cv in &input.channels {
            let chan =
                ohd_storage_core::registry::resolve_channel(&tx, etype.id, &cv.channel_path)?;
            let (vr, vi, vt, ve): (Option<f64>, Option<i64>, Option<String>, Option<i32>) =
                match &cv.value {
                    ohd_storage_core::events::ChannelScalar::Real { real_value } => {
                        (Some(*real_value), None, None, None)
                    }
                    ohd_storage_core::events::ChannelScalar::Int { int_value } => {
                        (None, Some(*int_value), None, None)
                    }
                    ohd_storage_core::events::ChannelScalar::Bool { bool_value } => {
                        (None, Some(*bool_value as i64), None, None)
                    }
                    ohd_storage_core::events::ChannelScalar::Text { text_value } => {
                        (None, None, Some(text_value.clone()), None)
                    }
                    ohd_storage_core::events::ChannelScalar::EnumOrdinal { enum_ordinal } => {
                        (None, None, None, Some(*enum_ordinal))
                    }
                };
            tx.execute(
                "INSERT INTO event_channels
                    (event_id, channel_id, value_real, value_int, value_text, value_enum)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![event_id, chan.id, vr, vi, vt, ve],
            )?;
        }

        // Mark the pending row as approved + link to the new event.
        let now = ohd_storage_core::audit::now_ms();
        tx.execute(
            "UPDATE pending_events
                SET status = 'approved', reviewed_at_ms = ?1, approved_event_id = ?2
              WHERE id = ?3",
            params![now, event_id, pending_id],
        )?;

        // Audit: one row for the approve, with the grant_id of the submitter.
        ohd_storage_core::audit::append(
            &tx,
            &ohd_storage_core::audit::AuditEntry {
                ts_ms: now,
                actor_type: ohd_storage_core::audit::ActorType::Self_,
                auto_granted: false,
                grant_id: Some(grant_id),
                action: "pending_approve".into(),
                query_kind: Some("approve_pending".into()),
                query_params_json: Some(format!(
                    "{{\"pending_ulid\":{ulid_str:?},\"event_id\":{event_id}}}"
                )),
                rows_returned: None,
                rows_filtered: None,
                result: ohd_storage_core::audit::AuditResult::Success,
                reason: None,
                caller_ip: None,
                caller_ua: None,
                delegated_for_user_ulid: None,
            },
        )?;

        tx.commit()?;
        Ok(event_id)
    })?;
    println!("approved {ulid_str} (event_id={event_id})");
    Ok(())
}

fn parse_key(s: Option<&str>) -> anyhow::Result<Vec<u8>> {
    match s {
        None => Ok(vec![]),
        Some(s) if s.is_empty() => Ok(vec![]),
        Some(s) => Ok(hex::decode(s).map_err(|e| anyhow::anyhow!("--cipher-key not hex: {e}"))?),
    }
}

fn init_tracing() {
    use tracing_subscriber::{fmt, prelude::*, EnvFilter};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(false))
        .try_init();
}

/// Ensure the registry has `std.clinical_note` + its `text` and `author`
/// channels. The canonical migration ships a smaller registry (glucose, HR,
/// temp, BP, meds, symptoms, meals, mood); the OHD Care demo writes
/// `std.clinical_note` from the doctor side. We INSERT OR IGNORE the rows so
/// rerunning is a no-op once they exist.
///
/// The clinical-note type lands in `migrations/003_clinical_note.sql` once
/// the storage v1.x pass adds it; this helper goes away then.
fn seed_clinical_note_type_if_missing(storage: &Storage) -> anyhow::Result<()> {
    storage.with_conn(|conn| -> ohd_storage_core::Result<()> {
        // Insert event_type. SQLite returns the rowid via last_insert_rowid()
        // only on a successful INSERT — for INSERT OR IGNORE that no-ops, we
        // re-query to find the existing id.
        conn.execute(
            "INSERT OR IGNORE INTO event_types
                (namespace, name, description, default_sensitivity_class)
             VALUES ('std', 'clinical_note',
                     'Clinical note authored by an operator (Care side write target)',
                     'medical_clinical')",
            [],
        )?;
        let event_type_id: i64 = conn.query_row(
            "SELECT id FROM event_types WHERE namespace = 'std' AND name = 'clinical_note'",
            [],
            |r| r.get(0),
        )?;

        // Channels: `text` (free text body) and `author` (display name of
        // operator). Both `value_type='text'`. INSERT OR IGNORE on the
        // (event_type_id, path) unique key. The `name` column is the
        // human-readable channel label and is also the path leaf for these
        // top-level channels, so `name == path` here.
        for path in ["text", "author"] {
            conn.execute(
                "INSERT OR IGNORE INTO channels
                    (event_type_id, parent_id, name, path, value_type, sensitivity_class)
                 VALUES (?1, NULL, ?2, ?2, 'text', 'medical_clinical')",
                params![event_type_id, path],
            )?;
        }
        Ok(())
    })?;
    Ok(())
}
