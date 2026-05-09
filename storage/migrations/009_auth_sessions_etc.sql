-- OHD Storage: AuthService extensions — sessions / invites / device tokens /
-- push registrations / notification config.
--
-- Per `spec/auth.md` and `proto/ohdc/v0/auth.proto`. The session-shaped data
-- already lives on `_tokens` (token_prefix='ohds') from migration 001; this
-- migration adds light extension columns + four new tables for the rest of
-- the AuthService surface.
--
-- New tables:
--   - `_pending_invites`         — out-of-band invitations (delegate / read-share).
--   - `_device_token_grants`     — device token allowlist of event types.
--   - `_push_registrations`      — FCM/APNs/web push tokens for push-wake.
--   - `_notification_config`     — singleton row of per-user notification prefs.
--
-- Columns added to `_tokens`:
--   - `last_seen_ms`   — touched by a future "session ping" path.
--   - `user_agent`     — user-agent at issuance time (best-effort).
--   - `ip_origin`      — string-form IP at issuance time (best-effort).
--
-- All idempotent.

-- =============================================================================
-- Sessions: extend `_tokens` with introspection fields.
-- =============================================================================

ALTER TABLE _tokens ADD COLUMN last_seen_ms INTEGER;
ALTER TABLE _tokens ADD COLUMN user_agent TEXT;
ALTER TABLE _tokens ADD COLUMN ip_origin TEXT;

-- =============================================================================
-- Invites: out-of-band redemption tokens.
-- =============================================================================

CREATE TABLE IF NOT EXISTS _pending_invites (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    ulid_random       BLOB NOT NULL UNIQUE,
    invite_token_hash BLOB NOT NULL UNIQUE,         -- SHA-256(invite cleartext)
    issuer_user_ulid  BLOB NOT NULL,
    email_bound       TEXT,                         -- optional bound email
    note              TEXT,                         -- free-text description
    issued_at_ms      INTEGER NOT NULL,
    expires_at_ms     INTEGER,
    redeemed_at_ms    INTEGER,
    redeemed_by_user_ulid BLOB,
    revoked_at_ms     INTEGER
);

CREATE INDEX IF NOT EXISTS idx_invites_issuer ON _pending_invites (issuer_user_ulid);
CREATE INDEX IF NOT EXISTS idx_invites_expires ON _pending_invites (expires_at_ms)
    WHERE redeemed_at_ms IS NULL AND revoked_at_ms IS NULL;

-- =============================================================================
-- Device tokens: per-grant event-type allowlist.
--
-- `_tokens` holds the bearer hash + grant_id; this table augments the
-- corresponding `grants` row with the device-specific metadata: kind label
-- + the allowed-event-types CSV. The grant write-side rules already enforce
-- which event types are accepted on PutEvents, so this is a discoverability
-- + introspection helper rather than an enforcement boundary.
-- =============================================================================

CREATE TABLE IF NOT EXISTS _device_token_grants (
    grant_id          INTEGER PRIMARY KEY REFERENCES grants(id) ON DELETE CASCADE,
    device_label      TEXT NOT NULL,
    device_kind       TEXT NOT NULL,
    event_types_csv   TEXT NOT NULL DEFAULT '',
    issued_at_ms      INTEGER NOT NULL
);

-- =============================================================================
-- Push registrations: FCM/APNs/web-push tokens.
-- =============================================================================

CREATE TABLE IF NOT EXISTS _push_registrations (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    ulid_random       BLOB NOT NULL UNIQUE,
    user_ulid         BLOB NOT NULL,
    platform          TEXT NOT NULL,                -- 'fcm' | 'apns' | 'web' | 'email'
    push_token        TEXT NOT NULL,
    registered_at_ms  INTEGER NOT NULL,
    last_seen_ms      INTEGER,
    revoked_at_ms     INTEGER,
    UNIQUE (platform, push_token)
);

CREATE INDEX IF NOT EXISTS idx_push_user
    ON _push_registrations (user_ulid)
    WHERE revoked_at_ms IS NULL;

-- =============================================================================
-- Notification config: singleton row per user.
-- =============================================================================

CREATE TABLE IF NOT EXISTS _notification_config (
    user_ulid             BLOB PRIMARY KEY,
    quiet_hours_enabled   INTEGER NOT NULL DEFAULT 0,
    quiet_hours_start     INTEGER,                  -- 0..23 (local hour)
    quiet_hours_end       INTEGER,                  -- 0..23 (local hour)
    quiet_hours_tz        TEXT,                     -- IANA zone, e.g. "Europe/Prague"
    updated_at_ms         INTEGER NOT NULL
);

-- =============================================================================
-- Attachments: per-blob wrapped DEK column for filesystem-level encryption.
--
-- Existing attachments rows have wrapped_dek = NULL → plaintext on disk
-- (back-compat). New writes populate this column with a per-attachment DEK
-- wrapped under K_envelope; the on-disk blob is AES-256-GCM ciphertext.
-- =============================================================================

ALTER TABLE attachments ADD COLUMN wrapped_dek BLOB;
ALTER TABLE attachments ADD COLUMN dek_nonce BLOB;
