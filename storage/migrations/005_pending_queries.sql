-- OHD Storage: per-query approval queue.
--
-- Backs the `grants.require_approval_per_query` policy from
-- `spec/storage-format.md`/`spec/privacy-access.md`. When a grant has that
-- flag set, every read query lands in this table instead of executing
-- immediately. The user's Connect app reviews each pending query and either
-- approves (the query then re-runs and returns data) or rejects (returns
-- OUT_OF_SCOPE / APPROVAL_TIMEOUT).
--
-- This is structurally separate from `pending_events` (which is the
-- write-with-approval queue): writes carry a payload to commit on approval;
-- reads carry a *query* to execute on approval. The shapes don't overlap
-- enough to warrant fusion.

CREATE TABLE IF NOT EXISTS pending_queries (
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,
  ulid_random        BLOB NOT NULL UNIQUE,
  grant_id           INTEGER NOT NULL REFERENCES grants(id) ON DELETE CASCADE,
  query_kind         TEXT NOT NULL,            -- 'query_events' | 'aggregate' | 'correlate' | 'read_samples' | 'read_attachment' | 'get_event_by_ulid'
  query_hash         BLOB NOT NULL,            -- sha256 of canonical query_payload (for dedup / audit join)
  query_payload      TEXT NOT NULL,            -- canonical JSON payload of the original request
  requested_at_ms    INTEGER NOT NULL,
  expires_at_ms      INTEGER NOT NULL,         -- auto-expire after this; status becomes 'expired'
  decided_at_ms      INTEGER,
  decision           TEXT NOT NULL DEFAULT 'pending',  -- 'pending' | 'approved' | 'rejected' | 'expired'
  decided_by_actor_id INTEGER                  -- decided_by_actor_id is the rowid of the self-session token row in _tokens (best-effort)
);

CREATE INDEX IF NOT EXISTS idx_pending_queries_status
  ON pending_queries (decision, requested_at_ms);
CREATE INDEX IF NOT EXISTS idx_pending_queries_grant
  ON pending_queries (grant_id, decision);
