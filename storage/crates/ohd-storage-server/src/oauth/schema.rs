//! Idempotent DDL bootstrap for the `oauth_*` tables.
//!
//! The canonical schema lives in `migrations/012_oauth_state.sql`, but adding
//! that migration to `crates/ohd-storage-core/src/format.rs::MIGRATIONS` is
//! the core agent's territory (per this pass's constraint set). To stay
//! self-contained we run the same DDL idempotently the first time
//! [`bootstrap`] is called from the OAuth wiring path. Once the migration
//! lands properly in `format.rs`, the call here becomes a no-op.

use ohd_storage_core::storage::Storage;
use ohd_storage_core::Result;

/// Run the OAuth state DDL. Idempotent; safe to call on every server start.
pub fn bootstrap(storage: &Storage) -> Result<()> {
    storage.with_conn_mut(|conn| {
        let tx = conn.transaction()?;
        tx.execute_batch(DDL)?;
        tx.commit()?;
        Ok(())
    })
}

/// The DDL block. Identical to `migrations/012_oauth_state.sql`.
const DDL: &str = r#"
CREATE TABLE IF NOT EXISTS oauth_clients (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    client_id          TEXT NOT NULL UNIQUE,
    client_name        TEXT NOT NULL,
    client_secret_hash BLOB,
    redirect_uris      TEXT NOT NULL,
    grant_types_csv    TEXT NOT NULL,
    response_types_csv TEXT NOT NULL,
    created_at_ms      INTEGER NOT NULL,
    created_by_user_ulid BLOB
);
CREATE INDEX IF NOT EXISTS idx_oauth_clients_client_id ON oauth_clients(client_id);

CREATE TABLE IF NOT EXISTS oauth_signing_keys (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    kid               TEXT NOT NULL UNIQUE,
    alg               TEXT NOT NULL DEFAULT 'RS256',
    private_key_pem   BLOB NOT NULL,
    public_jwk_json   TEXT NOT NULL,
    wrap_alg          TEXT,
    nonce             BLOB,
    created_at_ms     INTEGER NOT NULL,
    rotated_at_ms     INTEGER
);
CREATE INDEX IF NOT EXISTS idx_oauth_signing_keys_active
    ON oauth_signing_keys(rotated_at_ms) WHERE rotated_at_ms IS NULL;

CREATE TABLE IF NOT EXISTS oauth_authorization_codes (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    code_hash             BLOB NOT NULL UNIQUE,
    client_id             TEXT NOT NULL,
    user_ulid             BLOB NOT NULL,
    redirect_uri          TEXT NOT NULL,
    scope                 TEXT NOT NULL,
    code_challenge        TEXT NOT NULL,
    code_challenge_method TEXT NOT NULL,
    issued_at_ms          INTEGER NOT NULL,
    expires_at_ms         INTEGER NOT NULL,
    used_at_ms            INTEGER
);
CREATE INDEX IF NOT EXISTS idx_oauth_authorization_codes_lookup
    ON oauth_authorization_codes(code_hash);

CREATE TABLE IF NOT EXISTS oauth_device_codes (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    device_code_hash       BLOB NOT NULL UNIQUE,
    user_code              TEXT NOT NULL UNIQUE,
    client_id              TEXT NOT NULL,
    scope                  TEXT NOT NULL,
    issued_at_ms           INTEGER NOT NULL,
    expires_at_ms          INTEGER NOT NULL,
    completed_at_ms        INTEGER,
    completing_user_ulid   BLOB,
    redeemed_at_ms         INTEGER,
    last_polled_at_ms      INTEGER
);
CREATE INDEX IF NOT EXISTS idx_oauth_device_codes_user_code
    ON oauth_device_codes(user_code);

CREATE TABLE IF NOT EXISTS oauth_refresh_tokens (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    refresh_token_hash BLOB NOT NULL UNIQUE,
    client_id         TEXT NOT NULL,
    user_ulid         BLOB NOT NULL,
    scope             TEXT NOT NULL,
    issued_at_ms      INTEGER NOT NULL,
    expires_at_ms     INTEGER NOT NULL,
    revoked_at_ms     INTEGER
);
CREATE INDEX IF NOT EXISTS idx_oauth_refresh_tokens_lookup
    ON oauth_refresh_tokens(refresh_token_hash);
"#;
