-- OHD Storage: per-channel end-to-end encryption for sensitive sensitivity classes.
--
-- Per `spec/encryption.md` "End-to-end channel encryption (deferred)" + the
-- v1.x scope split agreed in STATUS.md: the storage daemon already gates
-- `event_channels.value_*` columns under SQLCipher whole-file encryption, but
-- a compromised daemon (or operator with access to the unlocked file) can
-- still read the value bytes for *every* channel — including `mental_health`,
-- `sexual_health`, `substance_use`, `reproductive`. This migration adds a
-- value-level encryption layer keyed off a per-class DEK (`K_class`) that the
-- daemon only holds in RAM during a write/read transaction.
--
-- Threat model:
--   - Operator (e.g. OHD Cloud) handles your data BUT can't read mental_health
--     entries without the user's K_envelope (which the daemon receives only
--     during an active session).
--   - Subpoena at the operator level recovers ciphertext only for these
--     classes.
--   - Grant tokens to clinicians who need access carry the wrapping key
--     material (see `class_key_wraps` BLOB column on `grants`).
--
-- Key hierarchy (recap, see `crates/ohd-storage-core/src/encryption.rs`):
--
--     K_recovery (BIP39, user-held; never on storage daemon)
--         └─> K_envelope (HKDF-derived; daemon receives during session)
--                 └─> K_class[mental_health] (AES-256 DEK, wrapped under
--                     K_envelope, stored in `class_keys.wrapped_key`)
--                 └─> K_class[sexual_health]
--                 └─> K_class[substance_use]
--                 └─> K_class[reproductive]
--
-- v1 simplification (see STATUS.md "v1 scope split"): a deterministic
-- K_envelope is derived from the SQLCipher key (`K_file`) at first open via
-- HKDF-SHA256. The full K_recovery / BIP39 / multi-device handoff hierarchy is
-- still v1.x; what this migration enables is the *value-level encryption
-- pipeline* against the day a real K_envelope replaces the deterministic one.
-- The pipeline is identical regardless of where K_envelope comes from.

-- =============================================================================
-- Per-class data-encryption keys (DEKs).
--
-- One row per sensitivity class that has encryption enabled. Encrypted classes
-- live in the configurable allowlist (default: mental_health, sexual_health,
-- substance_use, reproductive). Adding a class is a no-downtime operation —
-- the daemon lazily derives a fresh DEK on first write.
-- =============================================================================

CREATE TABLE IF NOT EXISTS class_keys (
    sensitivity_class  TEXT PRIMARY KEY,
    wrapped_key        BLOB NOT NULL,                 -- the per-class DEK encrypted under K_envelope
    wrap_alg           TEXT NOT NULL DEFAULT 'aes-256-gcm',
    nonce              BLOB NOT NULL,                 -- 12-byte AES-GCM nonce
    created_at_ms      INTEGER NOT NULL,
    rotated_at_ms      INTEGER                        -- non-null if this row was retired; new writes use the latest non-rotated key per class
);

-- =============================================================================
-- Key history for rotation.
--
-- Each `event_channels.value_blob` row carries a `key_id` pointing to a row
-- here (NOT the live `class_keys` row, since that gets rotated). New writes
-- always use the latest non-rotated key per class; reads look up by `key_id`.
-- =============================================================================

CREATE TABLE IF NOT EXISTS class_key_history (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    sensitivity_class  TEXT NOT NULL,
    wrapped_key        BLOB NOT NULL,                 -- copy of class_keys.wrapped_key at the time the row was active
    nonce              BLOB NOT NULL,                 -- 12-byte AES-GCM nonce used to wrap the DEK
    created_at_ms      INTEGER NOT NULL,
    rotated_at_ms      INTEGER                        -- non-null = retired; old reads still resolve via this row
);

CREATE INDEX IF NOT EXISTS idx_class_key_history_class
    ON class_key_history (sensitivity_class, id DESC);

-- =============================================================================
-- event_channels gains optional encryption columns.
--
-- When a channel value is stored encrypted, the `value_*` plaintext columns
-- are NULL and `value_blob` holds the AES-GCM ciphertext (12-byte nonce +
-- 16-byte tag + ciphertext bytes); `encryption_key_id` references the
-- `class_key_history.id` that wraps the DEK used.
-- =============================================================================

ALTER TABLE event_channels ADD COLUMN encrypted          INTEGER NOT NULL DEFAULT 0;
ALTER TABLE event_channels ADD COLUMN value_blob         BLOB;
ALTER TABLE event_channels ADD COLUMN encryption_key_id  INTEGER REFERENCES class_key_history(id);

CREATE INDEX IF NOT EXISTS idx_event_channels_encrypted
    ON event_channels (encrypted) WHERE encrypted = 1;

-- =============================================================================
-- grants gains a wrap-material column.
--
-- When a grant scope includes encrypted classes, `class_key_wraps` carries a
-- CBOR-encoded map `{sensitivity_class -> wrapped_K_class}` so the grantee
-- (running their own storage handle) can unwrap each DEK under their own
-- K_envelope. v1 only handles the single-storage case (the user's grants on
-- their own storage); multi-storage grant scenarios are documented as v0.x in
-- STATUS.md.
-- =============================================================================

ALTER TABLE grants ADD COLUMN class_key_wraps BLOB;
