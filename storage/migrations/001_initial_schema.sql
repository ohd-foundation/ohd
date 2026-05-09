-- OHD Storage: initial schema (format_version "1.0").
--
-- Mirrors `spec/storage-format.md` "SQL schema". Idempotent against an empty
-- database; the migration runner gates on `_meta.format_version`.
--
-- Pragmas are applied programmatically by the open path; SQLCipher key is set
-- before this script runs.

-- =============================================================================
-- Meta
-- =============================================================================

CREATE TABLE IF NOT EXISTS _meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

-- =============================================================================
-- Registry
-- =============================================================================

CREATE TABLE IF NOT EXISTS event_types (
  id                        INTEGER PRIMARY KEY,
  namespace                 TEXT NOT NULL,
  name                      TEXT NOT NULL,
  description               TEXT,
  schema_version            INTEGER NOT NULL DEFAULT 1,
  default_sensitivity_class TEXT NOT NULL DEFAULT 'general',
  UNIQUE (namespace, name)
);

CREATE TABLE IF NOT EXISTS channels (
  id                INTEGER PRIMARY KEY,
  event_type_id     INTEGER NOT NULL REFERENCES event_types(id),
  parent_id         INTEGER REFERENCES channels(id),
  name              TEXT NOT NULL,
  path              TEXT NOT NULL,
  display_name      TEXT,
  value_type        TEXT NOT NULL,
  unit              TEXT,
  enum_values       TEXT,
  is_required       INTEGER NOT NULL DEFAULT 0,
  sensitivity_class TEXT NOT NULL DEFAULT 'general',
  UNIQUE (event_type_id, path)
);

CREATE INDEX IF NOT EXISTS idx_channels_parent ON channels (parent_id);

CREATE TABLE IF NOT EXISTS type_aliases (
  old_namespace     TEXT NOT NULL,
  old_name          TEXT NOT NULL,
  new_event_type_id INTEGER NOT NULL REFERENCES event_types(id),
  PRIMARY KEY (old_namespace, old_name)
);

CREATE TABLE IF NOT EXISTS channel_aliases (
  event_type_id  INTEGER NOT NULL REFERENCES event_types(id),
  old_path       TEXT NOT NULL,
  new_channel_id INTEGER NOT NULL REFERENCES channels(id),
  PRIMARY KEY (event_type_id, old_path)
);

-- =============================================================================
-- Provenance
-- =============================================================================

CREATE TABLE IF NOT EXISTS devices (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  kind          TEXT NOT NULL,
  vendor        TEXT,
  model         TEXT,
  serial_or_id  TEXT,
  metadata_json TEXT,
  UNIQUE (kind, vendor, model, serial_or_id)
);

CREATE TABLE IF NOT EXISTS app_versions (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  app_name  TEXT NOT NULL,
  version   TEXT NOT NULL,
  platform  TEXT,
  UNIQUE (app_name, version, platform)
);

CREATE TABLE IF NOT EXISTS peer_sync (
  id                       INTEGER PRIMARY KEY AUTOINCREMENT,
  peer_label               TEXT NOT NULL UNIQUE,
  peer_kind                TEXT NOT NULL,
  peer_ulid                BLOB,
  last_outbound_rowid      INTEGER NOT NULL DEFAULT 0,
  last_inbound_peer_rowid  INTEGER NOT NULL DEFAULT 0,
  last_sync_started_ms     INTEGER,
  last_sync_ok_ms          INTEGER,
  last_status              TEXT
);

-- =============================================================================
-- Events
-- =============================================================================

CREATE TABLE IF NOT EXISTS events (
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,
  ulid_random        BLOB NOT NULL,
  timestamp_ms       INTEGER NOT NULL,
  tz_offset_minutes  INTEGER,
  tz_name            TEXT,
  duration_ms        INTEGER,
  event_type_id      INTEGER NOT NULL REFERENCES event_types(id),
  device_id          INTEGER REFERENCES devices(id),
  app_id             INTEGER REFERENCES app_versions(id),
  source             TEXT,
  source_id          TEXT,
  notes              TEXT,
  superseded_by      INTEGER REFERENCES events(id),
  origin_peer_id     INTEGER REFERENCES peer_sync(id),
  deleted_at_ms      INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_events_ulid ON events (ulid_random);
CREATE UNIQUE INDEX IF NOT EXISTS idx_events_dedup
  ON events (source, source_id) WHERE source_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_events_time
  ON events (timestamp_ms DESC) WHERE deleted_at_ms IS NULL;
CREATE INDEX IF NOT EXISTS idx_events_type_time
  ON events (event_type_id, timestamp_ms DESC) WHERE deleted_at_ms IS NULL;
CREATE INDEX IF NOT EXISTS idx_events_device_time
  ON events (device_id, timestamp_ms DESC) WHERE device_id IS NOT NULL AND deleted_at_ms IS NULL;

CREATE TABLE IF NOT EXISTS event_channels (
  event_id    INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
  channel_id  INTEGER NOT NULL REFERENCES channels(id),
  value_real  REAL,
  value_int   INTEGER,
  value_text  TEXT,
  value_enum  INTEGER,
  PRIMARY KEY (event_id, channel_id)
);

CREATE INDEX IF NOT EXISTS idx_channels_by_channel
  ON event_channels (channel_id, event_id);

CREATE TABLE IF NOT EXISTS event_samples (
  event_id      INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
  channel_id    INTEGER NOT NULL REFERENCES channels(id),
  block_index   INTEGER NOT NULL,
  t0_ms         INTEGER NOT NULL,
  t1_ms         INTEGER NOT NULL,
  sample_count  INTEGER NOT NULL,
  encoding      INTEGER NOT NULL,
  data          BLOB NOT NULL,
  PRIMARY KEY (event_id, channel_id, block_index)
);

CREATE INDEX IF NOT EXISTS idx_samples_time
  ON event_samples (channel_id, t0_ms);

CREATE TABLE IF NOT EXISTS attachments (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  ulid_random BLOB NOT NULL UNIQUE,
  event_id    INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
  sha256      BLOB NOT NULL,
  byte_size   INTEGER NOT NULL,
  mime_type   TEXT,
  filename    TEXT,
  encrypted   INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_attachments_event ON attachments (event_id);

-- =============================================================================
-- Cases
-- =============================================================================

CREATE TABLE IF NOT EXISTS cases (
  id                       INTEGER PRIMARY KEY AUTOINCREMENT,
  ulid_random              BLOB NOT NULL UNIQUE,
  case_type                TEXT NOT NULL,
  case_label               TEXT,
  started_at_ms            INTEGER NOT NULL,
  ended_at_ms              INTEGER,
  ended_by_grant_id        INTEGER,
  parent_case_id           INTEGER REFERENCES cases(id),
  predecessor_case_id      INTEGER REFERENCES cases(id),
  opening_authority_grant_id INTEGER,
  inactivity_close_after_h INTEGER,
  last_activity_at_ms      INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_cases_active     ON cases (ended_at_ms) WHERE ended_at_ms IS NULL;
CREATE INDEX IF NOT EXISTS idx_cases_parent     ON cases (parent_case_id) WHERE parent_case_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_cases_predecessor ON cases (predecessor_case_id) WHERE predecessor_case_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS case_filters (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  case_id           INTEGER NOT NULL REFERENCES cases(id) ON DELETE CASCADE,
  filter_json       TEXT NOT NULL,
  filter_label      TEXT,
  added_at_ms       INTEGER NOT NULL,
  added_by_grant_id INTEGER,
  removed_at_ms     INTEGER
);

CREATE INDEX IF NOT EXISTS idx_case_filters_case ON case_filters (case_id) WHERE removed_at_ms IS NULL;

CREATE TABLE IF NOT EXISTS case_reopen_tokens (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  ulid_random       BLOB NOT NULL UNIQUE,
  case_id           INTEGER NOT NULL REFERENCES cases(id),
  authority_grant_id INTEGER NOT NULL,
  issued_at_ms      INTEGER NOT NULL,
  expires_at_ms     INTEGER NOT NULL,
  used_at_ms        INTEGER,
  revoked_at_ms     INTEGER
);

CREATE INDEX IF NOT EXISTS idx_reopen_active ON case_reopen_tokens (case_id) WHERE used_at_ms IS NULL AND revoked_at_ms IS NULL;

-- =============================================================================
-- Grants
-- =============================================================================

CREATE TABLE IF NOT EXISTS grants (
  id                         INTEGER PRIMARY KEY AUTOINCREMENT,
  ulid_random                BLOB NOT NULL UNIQUE,
  grantee_label              TEXT NOT NULL,
  grantee_kind               TEXT NOT NULL,
  grantee_ulid               BLOB,
  is_template                INTEGER NOT NULL DEFAULT 0,
  created_at_ms              INTEGER NOT NULL,
  expires_at_ms              INTEGER,
  revoked_at_ms              INTEGER,
  purpose                    TEXT,
  default_action             TEXT NOT NULL DEFAULT 'deny',
  aggregation_only           INTEGER NOT NULL DEFAULT 0,
  strip_notes                INTEGER NOT NULL DEFAULT 1,
  require_approval_per_query INTEGER NOT NULL DEFAULT 0,
  approval_mode              TEXT NOT NULL DEFAULT 'always',
  notify_on_access           INTEGER NOT NULL DEFAULT 0,
  max_queries_per_day        INTEGER,
  max_queries_per_hour       INTEGER,
  rolling_window_days        INTEGER
);

CREATE INDEX IF NOT EXISTS idx_grants_active   ON grants (revoked_at_ms, expires_at_ms);
CREATE INDEX IF NOT EXISTS idx_grants_grantee  ON grants (grantee_ulid) WHERE grantee_ulid IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_grants_kind     ON grants (grantee_kind);
CREATE INDEX IF NOT EXISTS idx_grants_template ON grants (grantee_kind, is_template) WHERE is_template = 1;

CREATE TABLE IF NOT EXISTS grant_cases (
  grant_id    INTEGER NOT NULL REFERENCES grants(id) ON DELETE CASCADE,
  case_id     INTEGER NOT NULL REFERENCES cases(id),
  added_at_ms INTEGER NOT NULL,
  PRIMARY KEY (grant_id, case_id)
);
CREATE INDEX IF NOT EXISTS idx_grant_cases_case ON grant_cases (case_id);

CREATE TABLE IF NOT EXISTS grant_event_type_rules (
  grant_id      INTEGER NOT NULL REFERENCES grants(id) ON DELETE CASCADE,
  event_type_id INTEGER NOT NULL REFERENCES event_types(id),
  effect        TEXT NOT NULL,
  PRIMARY KEY (grant_id, event_type_id)
);

CREATE TABLE IF NOT EXISTS grant_channel_rules (
  grant_id    INTEGER NOT NULL REFERENCES grants(id) ON DELETE CASCADE,
  channel_id  INTEGER NOT NULL REFERENCES channels(id),
  effect      TEXT NOT NULL,
  PRIMARY KEY (grant_id, channel_id)
);

CREATE TABLE IF NOT EXISTS grant_sensitivity_rules (
  grant_id          INTEGER NOT NULL REFERENCES grants(id) ON DELETE CASCADE,
  sensitivity_class TEXT NOT NULL,
  effect            TEXT NOT NULL,
  PRIMARY KEY (grant_id, sensitivity_class)
);

CREATE TABLE IF NOT EXISTS grant_write_event_type_rules (
  grant_id      INTEGER NOT NULL REFERENCES grants(id) ON DELETE CASCADE,
  event_type_id INTEGER NOT NULL REFERENCES event_types(id),
  effect        TEXT NOT NULL,
  PRIMARY KEY (grant_id, event_type_id)
);

CREATE TABLE IF NOT EXISTS grant_auto_approve_event_types (
  grant_id      INTEGER NOT NULL REFERENCES grants(id) ON DELETE CASCADE,
  event_type_id INTEGER NOT NULL REFERENCES event_types(id),
  PRIMARY KEY (grant_id, event_type_id)
);

CREATE TABLE IF NOT EXISTS grant_time_windows (
  grant_id INTEGER PRIMARY KEY REFERENCES grants(id) ON DELETE CASCADE,
  from_ms  INTEGER,
  to_ms    INTEGER
);

-- =============================================================================
-- Pending events (write-with-approval queue)
-- =============================================================================

CREATE TABLE IF NOT EXISTS pending_events (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  ulid_random         BLOB NOT NULL UNIQUE,
  submitted_at_ms     INTEGER NOT NULL,
  submitting_grant_id INTEGER NOT NULL REFERENCES grants(id),
  payload_json        TEXT NOT NULL,
  status              TEXT NOT NULL,
  reviewed_at_ms      INTEGER,
  rejection_reason    TEXT,
  expires_at_ms       INTEGER NOT NULL,
  approved_event_id   INTEGER REFERENCES events(id)
);

CREATE INDEX IF NOT EXISTS idx_pending_status ON pending_events (status, submitted_at_ms);
CREATE INDEX IF NOT EXISTS idx_pending_grant  ON pending_events (submitting_grant_id, status);

-- =============================================================================
-- Audit
-- =============================================================================

CREATE TABLE IF NOT EXISTS audit_log (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  ts_ms             INTEGER NOT NULL,
  actor_type        TEXT NOT NULL,
  auto_granted      INTEGER NOT NULL DEFAULT 0,
  grant_id          INTEGER REFERENCES grants(id),
  action            TEXT NOT NULL,
  query_kind        TEXT,
  query_params_json TEXT,
  rows_returned     INTEGER,
  rows_filtered     INTEGER,
  result            TEXT NOT NULL,
  reason            TEXT,
  caller_ip         TEXT,
  caller_ua         TEXT
);

CREATE INDEX IF NOT EXISTS idx_audit_time   ON audit_log (ts_ms DESC);
CREATE INDEX IF NOT EXISTS idx_audit_grant  ON audit_log (grant_id, ts_ms DESC);
CREATE INDEX IF NOT EXISTS idx_audit_action ON audit_log (action, ts_ms DESC);

-- =============================================================================
-- Token store (per-file; spec/auth.md "system DB" keeps these out of the
-- per-user file in production, but for v1 single-binary deployments we colocate
-- them under a `_` table prefix to make the conformance smoke test self-
-- contained.)
-- =============================================================================

CREATE TABLE IF NOT EXISTS _tokens (
  id              INTEGER PRIMARY KEY AUTOINCREMENT,
  token_prefix    TEXT NOT NULL,             -- 'ohds' | 'ohdg' | 'ohdd'
  token_hash      BLOB NOT NULL UNIQUE,      -- SHA-256 of the bearer body
  user_ulid       BLOB NOT NULL,
  grant_id        INTEGER REFERENCES grants(id),
  issued_at_ms    INTEGER NOT NULL,
  expires_at_ms   INTEGER,
  revoked_at_ms   INTEGER,
  label           TEXT
);

CREATE INDEX IF NOT EXISTS idx_tokens_grant ON _tokens (grant_id);
