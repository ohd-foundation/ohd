-- OHD Storage: drop the V1 / V2 AAD-format discriminator columns.
--
-- The encryption codebase was flattened to a single value-side AEAD path
-- (XChaCha20-Poly1305 with the wide AAD spec'd in Codex review findings
-- #2 + #3). Migration 015 added `aad_version` columns on `event_channels`
-- and `attachments` to discriminate between the legacy V1 (AES-256-GCM,
-- narrow AAD) and V2 (XChaCha20-Poly1305, wide AAD) paths. With V1 removed
-- entirely, the discriminator column is dead weight — drop it.
--
-- Same for the value-side `wrap_alg` columns added in 015 (event_channels)
-- and 010 (attachments). The K_class / ECDH grant wraps remain AES-256-GCM
-- (low-volume, intentional) so `class_keys.wrap_alg` and the OAuth signing
-- key `wrap_alg` columns stay.
--
-- This is a no-op migration for any real deployment because there are no
-- production deployments — the previous V1/V2 dispatch never landed in
-- shipping code. The migration runs against fresh installs (no rows to
-- update) and against any local dev DBs that picked up migration 015.
--
-- SQLite >= 3.35 supports `ALTER TABLE … DROP COLUMN`. SQLCipher 4 ships
-- a 3.42+ vendored SQLite, so this works. The `DROP COLUMN` is irreversible
-- at the SQL layer — but the columns hold no decision-affecting data
-- (every encrypted blob in the codebase since the flatten is V2-shaped).
--
-- Idempotent: the migration ledger guarantees it's applied at most once;
-- `ALTER TABLE … DROP COLUMN` errors loudly if the column doesn't exist
-- (which would only happen in a corrupt schema).

ALTER TABLE event_channels DROP COLUMN aad_version;
ALTER TABLE event_channels DROP COLUMN wrap_alg;
ALTER TABLE attachments    DROP COLUMN aad_version;
ALTER TABLE attachments    DROP COLUMN wrap_alg;
