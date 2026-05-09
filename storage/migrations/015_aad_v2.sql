-- OHD Storage: AAD-format version markers for value-level AEAD.
--
-- Codex review findings #1, #2, #3:
--   - #1: AES-GCM 96-bit nonce birthday bound. Switch the *value-side* AEAD
--     (channel values + attachment payloads) to XChaCha20-Poly1305 (192-bit
--     nonce, collision-safe at any practical write volume).
--   - #2: Channel AAD too narrow. Bind `event_ulid` and `encryption_key_id`
--     into the AAD on every channel-value write so an operator can't replay
--     `(value_blob, encryption_key_id)` from one event onto another.
--   - #3: Attachment AAD doesn't bind `event_id`/`mime`/`filename`/`size`.
--     Bind all of them (plus `event_ulid`) into the per-attachment AAD on
--     every blob write.
--
-- Existing rows were written with the v1 AAD/algorithm; new rows go through
-- the v2 path. We discriminate by:
--   - `event_channels.aad_version` (NULL = v1, `2` = v2). The cipher choice
--     follows the version: v1 = AES-256-GCM, v2 = XChaCha20-Poly1305.
--   - `attachments.aad_version` (NULL = v1, `2` = v2) — same shape.
--   - `event_channels.wrap_alg` is added for symmetry with attachments
--     (existing behaviour stored algorithm only on the `class_keys` /
--     `attachments` rows; channels inherited the column-level convention).
--
-- The `class_keys.wrap_alg` column already exists (from `008_channel_encryption.sql`).
-- We don't change it: per-class DEK *wrap* stays AES-256-GCM (low write
-- volume, AAD already bound to the class). The XChaCha20 switch is
-- value-side only.
--
-- Idempotent: ALTER TABLE … ADD COLUMN runs once via the ledger.

ALTER TABLE event_channels ADD COLUMN aad_version INTEGER;
ALTER TABLE event_channels ADD COLUMN wrap_alg    TEXT;
ALTER TABLE attachments    ADD COLUMN aad_version INTEGER;
