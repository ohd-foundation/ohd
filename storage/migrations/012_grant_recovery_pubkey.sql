-- OHD Storage: multi-storage grant re-targeting (P1).
--
-- Per `spec/encryption.md` "Per-grant key sharing" + the v0.x deferral the
-- channel-encryption pass landed in STATUS.md ("Multi-storage grants — when
-- the grantee runs their own storage daemon, the wrap needs to be
-- re-targeted to a grantee-side K_envelope").
--
-- The pre-existing `grants.class_key_wraps` BLOB carries a CBOR map
-- `{sensitivity_class -> wrapped_K_class}`. For single-storage grants (the
-- user's own grants on their own storage) the wrap is under the issuer's
-- K_envelope and that's fine — the grantee unwraps with the same envelope.
-- For multi-storage grants (a clinician grant against a patient's storage)
-- we re-wrap each K_class via X25519 ECDH(K_recovery_seckey_issuer,
-- recovery_pubkey_grantee) → HKDF-SHA256 → wrap KEK.
--
-- This migration adds:
--   - `_meta.recovery_pubkey` row — published 32-byte X25519 pubkey for this
--     storage. Derived from K_recovery (BIP39) via HKDF-SHA256 with info
--     `b"ohd.v0.recovery_pubkey"`. Idempotent: a deterministic-key file
--     uses HKDF on K_file with the same info string. The seckey is held
--     only when the storage is unlocked; never persisted.
--   - `grants.grantee_recovery_pubkey BLOB` — when a grant was issued for a
--     remote grantee, holds the grantee's 32-byte recovery pubkey (the wrap
--     in `class_key_wraps` is under that pubkey via ECDH).
--   - `grants.issuer_recovery_pubkey BLOB` — published copy of this
--     storage's recovery pubkey at issue time, so the grantee's daemon can
--     ECDH against it without an out-of-band fetch.
--
-- Idempotent: ALTER … ADD COLUMN safe on first apply; the migration ledger
-- ensures it's applied at most once.

ALTER TABLE grants ADD COLUMN grantee_recovery_pubkey BLOB;
ALTER TABLE grants ADD COLUMN issuer_recovery_pubkey  BLOB;

-- Reserve the slot. The `recovery_pubkey` row is populated at first open by
-- the Rust side (`Storage::open` derives K_recovery_keypair → publishes the
-- pubkey).
INSERT OR IGNORE INTO _meta (key, value) VALUES ('recovery_pubkey', '');
