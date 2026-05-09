-- OHD Storage: source signing for high-trust integrations (P2).
--
-- Per `spec/docs/components/connect.md` "Source signing": Libre, Dexcom,
-- lab providers, hospital EHRs may sign their submissions with a per-
-- integration key so storage records "this glucose reading was signed by
-- Libre's key X". Protects against leaked-token attackers forging
-- integration writes (the leaked token alone is no longer sufficient — the
-- attacker would also need the integration's signing key).
--
-- Two tables land:
--
--   1. `signers` — operator-managed registry of integration public keys.
--      Operators register Libre / Dexcom / lab-provider public keys at
--      deployment time. Self-session-only RPCs:
--        - RegisterSigner(label, pem, sig_alg) → kid
--        - ListSigners()
--        - RevokeSigner(kid)
--
--   2. `event_signatures` — one row per signed event, joining via
--      `event_id`. Carries the algorithm, signer KID, and signature bytes
--      so QueryEvents can render "signed by Libre" badges (Connect / Care /
--      Emergency UI consume `signed_by` on the wire).
--
-- Algorithm choice: Ed25519 by default (compact, widely supported,
-- pure-Rust verification via `ed25519-dalek`). RS256 / ES256 are accepted
-- for compatibility with integrations that already use OAuth-style keys
-- (verified through `jsonwebtoken`'s primitives).
--
-- Canonical encoding for the to-be-signed bytes is **deterministic CBOR**
-- of a fixed shape `{ulid, timestamp_ms, event_type, channels:[...]}`.
-- See `crates/ohd-storage-core/src/source_signing.rs` "canonical_event_bytes"
-- for the exact field set + ordering.

CREATE TABLE IF NOT EXISTS signers (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    signer_kid             TEXT NOT NULL UNIQUE,         -- operator-assigned key id, e.g. "libre.eu.2026-01"
    signer_label           TEXT NOT NULL,                -- human label "Libre EU production"
    sig_alg                TEXT NOT NULL,                -- 'ed25519' | 'rs256' | 'es256'
    public_key_pem         TEXT NOT NULL,                -- PEM-encoded SubjectPublicKeyInfo
    registered_at_ms       INTEGER NOT NULL,
    revoked_at_ms          INTEGER,                      -- non-null = no longer accepted for new signed events
    registered_by_actor_id INTEGER                       -- FK soft-link to audit_log.actor (we don't enforce; audit row always exists)
);

CREATE INDEX IF NOT EXISTS idx_signers_active ON signers (revoked_at_ms) WHERE revoked_at_ms IS NULL;

CREATE TABLE IF NOT EXISTS event_signatures (
    event_id      INTEGER PRIMARY KEY REFERENCES events(id) ON DELETE CASCADE,
    sig_alg       TEXT NOT NULL,                         -- copy of signers.sig_alg at write time
    signer_kid    TEXT NOT NULL,                         -- the kid that was used (matches signers.signer_kid)
    signature     BLOB NOT NULL,                         -- raw signature bytes (Ed25519: 64 B; RS256: variable)
    signed_at_ms  INTEGER NOT NULL                       -- timestamp the row was inserted (server side)
);

CREATE INDEX IF NOT EXISTS idx_event_signatures_kid
    ON event_signatures (signer_kid);
