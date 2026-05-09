-- OHD Storage: per-peer attachment-blob delivery watermark.
--
-- Per `spec/sync-protocol.md` "Attachment payload sync", attachment metadata
-- syncs in the normal frame stream but the sidecar blob payload is pulled
-- separately (lazy / on-demand). Caches need to know which attachment ULIDs
-- they've already shipped to / pulled from a given peer so they don't redo
-- the bytes-on-the-wire on every pass. This table records the
-- (peer_id, attachment_id) couples that have crossed the wire.
--
-- Direction is captured by the `direction` column ('push' = caller pushed
-- the blob to the peer; 'pull' = caller pulled the blob from the peer).
-- The (peer_id, attachment_id, direction) triple is unique.
--
-- Rows are insert-only; clearing the table forces a re-sync of any
-- attachments that haven't been re-touched by either side.

CREATE TABLE IF NOT EXISTS peer_attachment_sync (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  peer_id         INTEGER NOT NULL REFERENCES peer_sync(id) ON DELETE CASCADE,
  attachment_id   INTEGER NOT NULL REFERENCES attachments(id) ON DELETE CASCADE,
  direction       TEXT NOT NULL,             -- 'push' | 'pull'
  delivered_at_ms INTEGER NOT NULL,
  byte_size       INTEGER NOT NULL,
  UNIQUE (peer_id, attachment_id, direction)
);

CREATE INDEX IF NOT EXISTS idx_peer_attachment_sync_peer
  ON peer_attachment_sync (peer_id, direction);
