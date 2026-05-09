//! On-disk file format: open/create, SQLCipher wiring, migration runner.
//!
//! Implements the schema in `spec/storage-format.md`. Migrations are embedded
//! at compile time via `include_str!`; the runner is a tiny `_meta.format_version`
//! gated apply-or-skip.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::{Error, Result, FORMAT_VERSION};

/// Embedded migration scripts, applied in declaration order. Each script must
/// be idempotent (uses `CREATE … IF NOT EXISTS`, `INSERT OR IGNORE`, etc.).
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "001_initial_schema",
        include_str!("../../../migrations/001_initial_schema.sql"),
    ),
    (
        "002_std_registry",
        include_str!("../../../migrations/002_std_registry.sql"),
    ),
    (
        "003_case_markers",
        include_str!("../../../migrations/003_case_markers.sql"),
    ),
    (
        "004_peer_attachment_sync",
        include_str!("../../../migrations/004_peer_attachment_sync.sql"),
    ),
    (
        "005_pending_queries",
        include_str!("../../../migrations/005_pending_queries.sql"),
    ),
    (
        "006_delegate_grants",
        include_str!("../../../migrations/006_delegate_grants.sql"),
    ),
    (
        "007_multi_identity",
        include_str!("../../../migrations/007_multi_identity.sql"),
    ),
    (
        "008_channel_encryption",
        include_str!("../../../migrations/008_channel_encryption.sql"),
    ),
    (
        "009_auth_sessions_etc",
        include_str!("../../../migrations/009_auth_sessions_etc.sql"),
    ),
    (
        "010_encrypted_attachments",
        include_str!("../../../migrations/010_encrypted_attachments.sql"),
    ),
    (
        "011_bip39_recovery",
        include_str!("../../../migrations/011_bip39_recovery.sql"),
    ),
    (
        "012_grant_recovery_pubkey",
        include_str!("../../../migrations/012_grant_recovery_pubkey.sql"),
    ),
    (
        "013_source_signing",
        include_str!("../../../migrations/013_source_signing.sql"),
    ),
    (
        "014_class_key_rotation_fk",
        include_str!("../../../migrations/014_class_key_rotation_fk.sql"),
    ),
    (
        "015_aad_v2",
        include_str!("../../../migrations/015_aad_v2.sql"),
    ),
    (
        "016_drop_v1",
        include_str!("../../../migrations/016_drop_v1.sql"),
    ),
    (
        "017_emergency_config",
        include_str!("../../../migrations/017_emergency_config.sql"),
    ),
];

/// One of the three deployment modes a per-user file can be in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeploymentMode {
    /// Canonical for the user; accepts external writes and grant queries.
    Primary,
    /// Mirrors a remote primary; cannot serve external grant queries.
    Cache,
    /// Read-only replica (backups, hot standbys).
    Mirror,
}

impl DeploymentMode {
    /// String form stored in `_meta.deployment_mode`.
    pub fn as_str(self) -> &'static str {
        match self {
            DeploymentMode::Primary => "primary",
            DeploymentMode::Cache => "cache",
            DeploymentMode::Mirror => "mirror",
        }
    }

    /// Parse the string form.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "primary" => Ok(DeploymentMode::Primary),
            "cache" => Ok(DeploymentMode::Cache),
            "mirror" => Ok(DeploymentMode::Mirror),
            other => Err(Error::InvalidArgument(format!(
                "unknown deployment mode: {other}"
            ))),
        }
    }
}

/// Opening parameters for [`open_or_create`].
pub struct OpenParams<'a> {
    /// Filesystem path to the user's `data.db`.
    pub path: &'a Path,
    /// SQLCipher key (32 bytes recommended). Empty means "open unencrypted",
    /// useful for testing only.
    pub cipher_key: &'a [u8],
    /// If true, create the file when missing; otherwise return [`Error::NotFound`].
    pub create_if_missing: bool,
    /// Deployment mode for newly-created files. Ignored when the file exists.
    pub create_mode: DeploymentMode,
    /// User ULID to stamp into `_meta.user_ulid` on creation. Ignored when the
    /// file exists.
    pub create_user_ulid: Option<[u8; 16]>,
}

/// Open an existing per-user file or create one and run migrations.
///
/// Connection state set on the returned handle:
/// - SQLCipher key applied (when `cipher_key` non-empty)
/// - `journal_mode=WAL`
/// - `foreign_keys=ON`
/// - `synchronous=NORMAL`
pub fn open_or_create(params: OpenParams<'_>) -> Result<(Connection, PathBuf)> {
    let path: PathBuf = params.path.to_path_buf();
    let exists = path.exists();
    if !exists && !params.create_if_missing {
        return Err(Error::NotFound);
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let conn = Connection::open(&path)?;
    apply_pragmas(&conn, params.cipher_key)?;

    if !exists {
        bootstrap_meta(&conn, params.create_mode, params.create_user_ulid)?;
    }
    run_migrations(&conn)?;

    // Verify the format version is one we understand.
    let on_disk: String = conn
        .query_row(
            "SELECT value FROM _meta WHERE key='format_version'",
            [],
            |r| r.get(0),
        )
        .unwrap_or_else(|_| FORMAT_VERSION.to_string());
    if on_disk != FORMAT_VERSION {
        return Err(Error::InvalidArgument(format!(
            "incompatible format_version on disk: {on_disk} (build expects {FORMAT_VERSION})"
        )));
    }

    Ok((conn, path))
}

fn apply_pragmas(conn: &Connection, cipher_key: &[u8]) -> Result<()> {
    if !cipher_key.is_empty() {
        // SQLCipher accepts either a passphrase ('x') or a 64-hex blob ('x"…"').
        // For binary keys we use the raw-key syntax: PRAGMA key = "x'<hex>'";
        let hex_key = hex::encode(cipher_key);
        conn.pragma_update(None, "key", format!("x'{hex_key}'"))?;
    }
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    Ok(())
}

fn bootstrap_meta(
    conn: &Connection,
    mode: DeploymentMode,
    user_ulid: Option<[u8; 16]>,
) -> Result<()> {
    // Make sure the _meta table exists before we INSERT into it. The 001
    // migration creates it with IF NOT EXISTS so we run only the first part.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )?;
    let now = now_ms();
    let user_ulid_hex = match user_ulid {
        Some(b) => hex::encode(b),
        None => hex::encode(crate::ulid::random_bytes(16)),
    };
    let inserts = [
        ("format_version", FORMAT_VERSION.to_string()),
        ("user_ulid", user_ulid_hex),
        ("deployment_mode", mode.as_str().to_string()),
        ("created_at_ms", now.to_string()),
        ("registry_version", "1".to_string()),
    ];
    for (k, v) in inserts {
        conn.execute(
            "INSERT OR IGNORE INTO _meta (key, value) VALUES (?1, ?2)",
            (k, v),
        )?;
    }
    Ok(())
}

fn run_migrations(conn: &Connection) -> Result<()> {
    // Tiny migration ledger inside _meta: rows like `mig:001_initial_schema = applied_at_ms`.
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
    )?;
    for (name, sql) in MIGRATIONS {
        let key = format!("mig:{name}");
        let already: Option<String> = conn
            .query_row(
                "SELECT value FROM _meta WHERE key = ?1",
                [key.as_str()],
                |r| r.get(0),
            )
            .ok();
        if already.is_some() {
            continue;
        }
        tracing::info!(name, "applying migration");
        conn.execute_batch(sql)?;
        conn.execute(
            "INSERT OR REPLACE INTO _meta (key, value) VALUES (?1, ?2)",
            (key, now_ms().to_string()),
        )?;
    }
    Ok(())
}

/// Convenience timestamp for code paths that don't depend on chrono.
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    d.as_millis() as i64
}
