-- OHD Storage: BIP39 K_recovery hierarchy.
--
-- Adds the per-file `k_recovery_salt` row to `_meta`. This salt binds the
-- derivation `BIP39 mnemonic → seed → HKDF-SHA256(salt, info=ohd.v0.file_key)
-- → K_file` to a specific file (two users with the same mnemonic don't
-- accidentally share file keys; defended in depth even though 24-word
-- collisions are statistically impossible).
--
-- Files created via `Storage::create()` (deterministic K_file path, v1)
-- leave this row unset. Files created via `Storage::create_with_mnemonic()`
-- write a fresh CSPRNG-generated 32-byte salt here at create time and a
-- `_meta.kdf_mode = 'bip39'` row marking the file as mnemonic-derived.
--
-- Per `spec/encryption.md` "Per-deployment-mode key flow" / "Recovery".
--
-- Idempotent: `_meta` is a key/value table; the migration just records the
-- migration ledger entry. Salts and modes are inserted by the create path
-- (Rust side); existing files keep working with their deterministic key.

-- No DDL changes — `_meta` already supports arbitrary key/value rows. This
-- migration's only purpose is to document the new `k_recovery_salt` and
-- `kdf_mode` keys + claim the version slot in the migration ledger so the
-- Rust create path can rely on the slot being present.

-- Reserve the slot. INSERT OR IGNORE so re-applying is a no-op.
INSERT OR IGNORE INTO _meta (key, value) VALUES ('kdf_mode', 'deterministic');
