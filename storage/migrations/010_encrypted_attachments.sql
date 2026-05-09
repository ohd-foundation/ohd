-- OHD Storage: filesystem-level encrypted attachments.
--
-- Per `spec/encryption.md` "Per-deployment-mode key flow" — each sidecar blob
-- on disk is encrypted with a per-attachment data-encryption key (DEK), with
-- the DEK wrapped under the per-user `K_envelope`. The wire-side AEAD format
-- is `nonce(12) || ciphertext_with_tag` written to disk in place of the raw
-- bytes.
--
-- The `attachments` table gained `wrapped_dek BLOB` and `dek_nonce BLOB` in
-- migration 009 (back-compat NULL = legacy plaintext blob). This migration
-- adds:
--
--   - `wrap_alg TEXT`     — algorithm tag (`aes-256-gcm` for v1; future-proofs
--                           for ChaCha20-Poly1305 fallback or KMS-wrapped DEK).
--                           NULL on legacy plaintext rows.
--   - `idx_attachments_encrypted` — partial index on rows pending lazy
--                                   migration / rotation, so the rekey worker
--                                   can scan them efficiently.
--
-- Idempotent: ALTER TABLE … ADD COLUMN is safe on first apply; the migration
-- ledger ensures it's applied at most once. `CREATE INDEX IF NOT EXISTS`
-- handles the index.

ALTER TABLE attachments ADD COLUMN wrap_alg TEXT;

CREATE INDEX IF NOT EXISTS idx_attachments_encrypted
  ON attachments (id)
  WHERE wrapped_dek IS NOT NULL;

CREATE INDEX IF NOT EXISTS idx_attachments_plaintext
  ON attachments (id)
  WHERE wrapped_dek IS NULL;
