-- Top-level flag for the open-ended event hierarchy
-- (food.eaten parent ↔ intake.* children, measurement.ecg_session parent ↔
-- per-second children, …).
--
-- Every event still gets its own ULID — no parent FK, no foreign-key tax,
-- no out-of-order-sync headache. The producer (FoodDetail, ECG importer)
-- just flips `top_level = 0` on detail rows. Grouping back into a parent
-- is done via a `correlation_id` channel when the UI actually needs it
-- (same pattern as food.consumption_started ↔ consumption_finished).
--
-- Defaults to `1` so every pre-existing row stays surfaced — no UI churn.

ALTER TABLE events ADD COLUMN top_level INTEGER NOT NULL DEFAULT 1;

-- History / Recent / home-count queries filter on this; an index on the
-- partial set keeps lookups O(log n) without bloating the file on detail
-- rows.
CREATE INDEX IF NOT EXISTS idx_events_top_level_time
    ON events (timestamp_ms DESC)
    WHERE top_level = 1 AND deleted_at_ms IS NULL;
