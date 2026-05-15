-- OHD SaaS schema — profiles, OIDC links, plans, payments.
--
-- Everything keyed by `profile_ulid` (a Crockford-base32 string the client
-- mints; opaque to us). Idempotent so the migration runner can replay.

CREATE TABLE IF NOT EXISTS profiles (
    profile_ulid       TEXT PRIMARY KEY,
    recovery_hash_hex  TEXT NOT NULL,           -- sha-256 of the 16x8 code, lowercase hex
    plan               TEXT NOT NULL DEFAULT 'free',
    created_at         TEXT NOT NULL,
    last_seen_at       TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS profiles_recovery_idx
    ON profiles(recovery_hash_hex);

CREATE TABLE IF NOT EXISTS oidc_identities (
    profile_ulid   TEXT NOT NULL,
    provider       TEXT NOT NULL,                -- e.g. 'https://accounts.google.com'
    sub            TEXT NOT NULL,                -- opaque from the issuer
    display_label  TEXT,
    linked_at      TEXT NOT NULL,
    PRIMARY KEY (provider, sub),
    FOREIGN KEY (profile_ulid) REFERENCES profiles(profile_ulid) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS oidc_profile_idx
    ON oidc_identities(profile_ulid);

CREATE TABLE IF NOT EXISTS payment_records (
    ulid                 TEXT PRIMARY KEY,
    profile_ulid         TEXT NOT NULL,
    created_at           TEXT NOT NULL,
    amount_minor_units   INTEGER NOT NULL,
    currency             TEXT NOT NULL,
    provider             TEXT NOT NULL,
    provider_charge_id   TEXT,
    status               TEXT NOT NULL,
    invoice_url          TEXT,
    FOREIGN KEY (profile_ulid) REFERENCES profiles(profile_ulid) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS payment_profile_idx
    ON payment_records(profile_ulid, created_at);

-- Tiny meta row so the runner can detect a populated DB.
CREATE TABLE IF NOT EXISTS _meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
INSERT OR IGNORE INTO _meta(key, value) VALUES ('schema_version', '1');
