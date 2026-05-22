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
-- oauth_authorization_codes + oauth_pending_logins hold only transient,
-- short-lived state (auth codes ~minutes, pending logins ~10 min). They are
-- dropped + recreated on every start so additive schema changes (the
-- client_nonce / nonce columns below) land on an already-deployed DB with no
-- migration; at most a few seconds of in-flight OAuth flows are lost.
DROP TABLE IF EXISTS oauth_authorization_codes;
DROP TABLE IF EXISTS oauth_pending_logins;

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
    nonce                 TEXT,                   -- the OHD client's OIDC `nonce`, echoed into the id_token
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

-- Pending OIDC-RP login flows. When a user picks an upstream provider on the
-- storage AS login page, the AS stashes the in-progress downstream
-- authorization request here, keyed by the random `oidc_state` it sends to
-- the provider. The `/oauth/oidc-callback` handler looks the row up by the
-- `state` the provider echoes back, completes the upstream exchange, and then
-- resumes the downstream authorization-code issuance.
CREATE TABLE IF NOT EXISTS oauth_pending_logins (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    oidc_state            TEXT NOT NULL UNIQUE,   -- state sent to the upstream provider
    oidc_nonce            TEXT NOT NULL,          -- nonce sent to the upstream provider
    pkce_verifier         TEXT NOT NULL,          -- our PKCE verifier toward the provider
    provider_key          TEXT NOT NULL,          -- catalog key (e.g. 'ohd_account')
    -- The downstream (OHD-client-facing) authorization request we must resume:
    client_id             TEXT NOT NULL,
    redirect_uri          TEXT NOT NULL,
    scope                 TEXT NOT NULL,
    client_state          TEXT NOT NULL,          -- the OHD client's own `state`
    code_challenge        TEXT NOT NULL,          -- the OHD client's PKCE challenge
    code_challenge_method TEXT NOT NULL,
    client_nonce          TEXT,                   -- the OHD client's OIDC `nonce`
    issued_at_ms          INTEGER NOT NULL,
    expires_at_ms         INTEGER NOT NULL,
    used_at_ms            INTEGER
);
CREATE INDEX IF NOT EXISTS idx_oauth_pending_logins_state
    ON oauth_pending_logins(oidc_state);
"#;
