-- OHD Storage: foreign key from `class_keys` to `class_key_history`.
--
-- Codex review finding #4 ("Class-key rotation drift"): the v1 implementation
-- maintained two independent SELECTs in `load_active_class_key`:
--   1. `SELECT wrapped_key FROM class_keys WHERE sensitivity_class = ?` to
--      unwrap the live DEK.
--   2. `SELECT id FROM class_key_history
--         WHERE sensitivity_class = ? AND rotated_at_ms IS NULL
--         ORDER BY id DESC LIMIT 1` to stamp the `key_id` onto fresh blobs.
--
-- A concurrent rotation (which also updates `class_keys` *and* inserts +
-- retires `class_key_history` rows) could observe a snapshot where the
-- "latest unrotated history row" is the new one but the live `class_keys`
-- row still holds the old wrapped DEK (or vice versa). The result is a
-- newly-written blob stamped with a `key_id` that doesn't actually wrap its
-- DEK — readable today, broken on the next process restart.
--
-- Fix: add `class_keys.current_history_id` as a foreign key into
-- `class_key_history(id)`. `bootstrap_class_keys` and `rotate_class_key`
-- maintain this linkage atomically (within the same transaction); reads
-- consult it as the single source of truth for which history row pairs
-- with the live `class_keys` row.
--
-- Idempotent: `ALTER TABLE … ADD COLUMN` is safe on first apply; the
-- migration ledger ensures it's run at most once.

ALTER TABLE class_keys
  ADD COLUMN current_history_id INTEGER REFERENCES class_key_history(id);

-- Backfill for existing class_keys rows. The original `bootstrap_class_keys`
-- implementation inserted matching `class_key_history` rows in the same
-- transaction with the same `(sensitivity_class, wrapped_key, nonce,
-- created_at_ms)`, so we can deterministically find the pairing. The
-- subquery is bounded to non-rotated history rows so a backfilled row is
-- never linked to a retired history record.
UPDATE class_keys
   SET current_history_id = (
     SELECT id FROM class_key_history
      WHERE class_key_history.sensitivity_class = class_keys.sensitivity_class
        AND class_key_history.rotated_at_ms IS NULL
      ORDER BY id DESC LIMIT 1
   )
 WHERE current_history_id IS NULL;
