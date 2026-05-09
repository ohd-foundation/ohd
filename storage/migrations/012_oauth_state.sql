-- OHD Storage: OAuth/OIDC IdP state.
--
-- These tables back the optional OAuth 2.0 + OIDC endpoints exposed by the
-- storage server when run with `--oauth-issuer <URL>` (see
-- `crates/ohd-storage-server/src/oauth.rs`). When the flag is unset the
-- storage instance never lights up these endpoints; the rows in the tables
-- below stay empty and have zero impact on the OHDC wire path.
--
-- Why colocate state in the per-user file: a self-hosted operator running a
-- single binary already pays the per-user-file cost; reusing the same DB for
-- the AS state keeps the moving parts to one. For multi-tenant OHD Cloud the
-- pattern would be one IdP DB at the deployment layer; that's a separate
-- (later) deliverable. Per-user-file is the v0 target — the storage daemon
-- runs a *single user's* IdP for the network identity of that one user.
--
-- All payloads (codes, tokens) are stored hashed (sha256), never in cleartext.
-- The cleartext is shown to the requesting client exactly once (per OAuth
-- standard).

-- Registered OAuth client applications (RFC 7591 minimal).
--
-- v0 supports:
--   - public clients (client_secret_hash IS NULL) for native/SPA apps using PKCE.
--   - confidential clients (client_secret_hash NOT NULL) for server-side apps.
CREATE TABLE IF NOT EXISTS oauth_clients (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    client_id          TEXT NOT NULL UNIQUE,
    client_name        TEXT NOT NULL,
    client_secret_hash BLOB,                 -- NULL → public client (PKCE required)
    redirect_uris      TEXT NOT NULL,        -- JSON array of strings
    grant_types_csv    TEXT NOT NULL,        -- e.g. "authorization_code,refresh_token,urn:ietf:params:oauth:grant-type:device_code"
    response_types_csv TEXT NOT NULL,        -- e.g. "code"
    created_at_ms      INTEGER NOT NULL,
    created_by_user_ulid BLOB                -- whichever self-session token registered this client (NULL for the bootstrap client)
);

CREATE INDEX IF NOT EXISTS idx_oauth_clients_client_id ON oauth_clients(client_id);

-- RS256 (and optionally ES256) signing keypair lifecycle.
--
-- The storage daemon holds at least one active key. id_tokens are signed with
-- the active key; rotation creates a new row and the old row keeps its public
-- side reachable via /oauth/jwks.json so previously-minted id_tokens still
-- verify until they expire.
--
-- `private_key_pem`: PKCS#1 PEM. Encrypted at rest under K_envelope when the
-- file has an envelope key (= production); plaintext otherwise (testing-only
-- no-cipher path). The encrypted form is `nonce(12) || ciphertext_with_tag`
-- under AES-256-GCM with AAD = `b"ohd.v0.oauth_signing_key:" || kid_bytes`.
CREATE TABLE IF NOT EXISTS oauth_signing_keys (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    kid               TEXT NOT NULL UNIQUE,
    alg               TEXT NOT NULL DEFAULT 'RS256',
    private_key_pem   BLOB NOT NULL,             -- encrypted-at-rest under K_envelope when wrap_alg = 'aes-256-gcm'
    public_jwk_json   TEXT NOT NULL,             -- the JWK the discovery doc exposes
    wrap_alg          TEXT,                      -- 'aes-256-gcm' if encrypted-at-rest; NULL otherwise
    nonce             BLOB,                      -- 12-byte AES-GCM nonce when wrap_alg is set
    created_at_ms     INTEGER NOT NULL,
    rotated_at_ms     INTEGER                    -- when this key was retired; NULL = active
);

CREATE INDEX IF NOT EXISTS idx_oauth_signing_keys_active
    ON oauth_signing_keys(rotated_at_ms) WHERE rotated_at_ms IS NULL;

-- Authorization codes (RFC 6749 §4.1 + RFC 7636 PKCE).
--
-- Single-use, short-lived (60 seconds default). `used_at_ms` flips to non-NULL
-- on first redemption — replay attempts after that point are rejected.
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

-- Device codes (RFC 8628).
--
-- Polled by the CLI client at the configured `interval` until the user
-- completes /oauth/device-confirm in a browser. `completing_user_ulid` is
-- written when the user confirms; the next poll then redeems the device_code
-- for tokens. After redemption, the row stays around for audit but completes
-- (no second redemption).
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

-- Refresh tokens.
--
-- Stored hashed; cleartext is shown once at issuance + once per refresh
-- (rotating refresh tokens is a v1.x deliverable; v0 keeps the original
-- refresh token alive across refreshes).
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
