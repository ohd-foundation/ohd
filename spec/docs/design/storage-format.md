# Design: Storage Format

> The on-disk format and storage primitives for OHD. This document supersedes
> `data-model.md` for *implementation* purposes; `data-model.md` remains the
> source of truth for the conceptual event vocabulary.

## Goals

1. **Portable.** The same file format works on a phone, a self-hosted VPS, and a SaaS deployment. No code path is platform-specific.
2. **Single-purpose.** A focused library with a typed event API. No SQL surface exposed to callers.
3. **Embeddable.** Library, not server. Single file per user. Backup is `cp`.
4. **Sparse-friendly.** Health data is irregular: glucose 4320×/day, BP a few times a week, mood once a week, "pimples under left arm" once a year. None of these should pay storage cost when not measured.
5. **Lossless and migration-safe.** No whole-file rewrites for schema changes; renames apply via aliases.
6. **Crash-safe.** Health data; corruption is unacceptable.
7. **Encryptable at rest.** Per-user key. A stolen file without the key reveals nothing.

## Key decisions (locked in)

| Decision | Choice | Why |
|---|---|---|
| Engine | **SQLite** with WAL | Crash-safe, embeddable, on every Android phone, mature SQLCipher for encryption. Domain library wraps it; SQL surface stays internal. |
| File layout | **One file per user** | Privacy isolation, trivial export/delete, per-user encryption keys, fits phone naturally. SaaS uses `users/<hash>/<user_id>/data.db`. |
| Identity | **`INTEGER PRIMARY KEY` (rowid) + 80-bit `ulid_random` BLOB** | Rowid is free in SQLite and gives compact FKs; ULID is the wire identity. |
| Time | **One `timestamp_ms` column** = measurement time = ULID time prefix | One source of truth. Insertion order survives in the rowid. |
| Mutability | **Immutable events** | Corrections are new events that reference the original via `superseded_by`. Audit-clean. |
| Channels | **EAV with a tree-structured registry** | No JSON in the hot path; sparse-friendly; queryable; tree captures grouped measurements (urine strip, BP, nutrition). |
| Dense series | **Compressed sample blocks**, ~15-min windows | One row per ~15 min instead of one per sample; ≥100× density. |
| Attachments | **Sidecar files**, addressed by SHA-256 | Big BLOBs hurt page cache and encryption cost. Per-user `blobs/` dir. |
| Migrations | **Type and channel aliases**, online compactor | No whole-file rewrites; aliases resolve at read time, compaction normalizes lazily. |
| Privacy | **Structural** (schema-level): sensitivity classes on types and channels + structured grant rules (event types, channels, sensitivity classes, time windows); deny wins | No per-event privacy annotations; the schema is the contract. All filtering is pure SQL; users can introspect what each grantee sees. |
| Deployment | Per-file **`deployment_mode`** = `primary` or `cache`; user picks at first launch | Same library, two topologies: local-primary (phone canonical) and remote-primary (server canonical, phone caches). Both shipped at launch. |
| Sync | **Bidirectional event-log replay** with ULID identity for dedup, per-peer watermarks in `peer_sync`, `origin_peer_id` to avoid echo | Immutability + ULID + tombstones = no merge logic. Wire framing is API-layer concern; storage exposes the primitives. |
| Concurrency | **Single writer + many readers** per file (WAL) | API process owns the writer; background compactor takes the lock briefly. |
| Encryption | **SQLCipher 4** with per-user key | Page-level encryption; key derived from user secret + (optional) server-wrapped material. |
| Transport | **HTTP/3 (QUIC) preferred, HTTP/2 fallback** | Fronted by Caddy; storage library is transport-agnostic. |

## File layout (per user)

```
users/<hash[:2]>/<user_ulid>/
├── data.db              # SQLite, SQLCipher-encrypted
├── data.db-wal          # WAL file (SQLite-managed)
├── data.db-shm          # Shared memory (SQLite-managed)
└── blobs/
    ├── <sha256-prefix>/<sha256>   # attachment payloads, per-user-key encrypted
    └── …
```

`<hash[:2]>` is the first two hex chars of `sha256(user_ulid)`, used to spread directories across the filesystem so no single dir holds 100k+ entries on a SaaS deployment. On a phone, the path simplifies to a single user dir.

## Identity

Every event has two identifiers:

- **Internal**: `id INTEGER PRIMARY KEY AUTOINCREMENT` — the SQLite rowid. Used for all FK references inside the file. Cheap, dense, deterministically encodes insertion order.
- **External (wire)**: a 128-bit ULID = `u48be(timestamp_ms) || ulid_random` where `ulid_random` is 10 bytes of CSPRNG. The ULID is what crosses the storage boundary — exports, sync, audit references, idempotency.

On disk only `ulid_random` is stored (10 bytes); the time prefix is reused from `timestamp_ms`. The ULID is reconstituted at the API boundary.

**Invariants**:

- `ulid_random` is unique within a file (`UNIQUE INDEX`). 80 bits of CSPRNG is collision-safe up to ~10¹² events at the same millisecond per writer; safe by 12+ orders of magnitude.
- For events with `timestamp_ms >= 0`, the ULID's time prefix equals `timestamp_ms`. Sorting by ULID = sorting by measurement time.
- For events with `timestamp_ms < 0` (pre-1970, e.g. childhood vaccination records being digitized), the ULID's time prefix is **clamped to 0**. The 80-bit random portion still guarantees unique identity. Chronological queries use `timestamp_ms` directly (signed comparison), which orders correctly across all eras; sort-by-ULID is undefined for mixed-era result sets and isn't relied on by any user-facing query.
- **Lookup by wire ID** extracts both halves and verifies them:
  - If the time prefix is **non-zero** (post-1970): `WHERE ulid_random = ? AND timestamp_ms = ?` — the time match is a free integrity check. Events are immutable, so a mismatch surfaces tampering, partial restores, importer bugs, or post-checksum bit-flips at the page boundary.
  - If the time prefix is **0** (clamp sentinel for pre-1970, or events at exact Unix epoch): `WHERE ulid_random = ?` — the time half is skipped. 80 bits of random remain a unique key on their own.
  The `UNIQUE INDEX idx_events_ulid` stays on `ulid_random` alone (sufficient for the dedup constraint); the time check is a verifier layered on top, not part of the unique key.
- Same-millisecond ULIDs are not deterministically ordered. Tie-breaking is undefined.

**Forensic property of the rowid**: because `INTEGER PRIMARY KEY AUTOINCREMENT` is monotonic by *insertion*, a backfilled event has a rowid larger than its measurement-time neighbors. This is a silent tamper-evidence signal: a row whose rowid is far ahead of its timestamp-neighbors was inserted out of order. The library doesn't expose rowid externally and doesn't promise this property to callers, but the data is in the file for forensics if ever needed.

## Time

One time column on each event:

- `timestamp_ms INTEGER NOT NULL` — **signed** Unix milliseconds, UTC. Negative values represent pre-1970 events (paper-record digitization, childhood histories of older patients). SQLite's `INTEGER` is already 64-bit signed; this is free at the storage layer. ULID minting clamps the time prefix to 0 for negative timestamps (see "Identity" invariants).
- `tz_offset_minutes INTEGER` — local offset at the time of measurement, e.g. `+120` for Prague summer. Optional but encouraged.
- `tz_name TEXT` — IANA zone name (`Europe/Prague`). Optional; carries the rule, not just the offset.
- `duration_ms INTEGER` — nullable; non-null for events with span (sleep, meal, exercise, hospital stay).

`timestamp_ms` is the ULID's time prefix and the primary sort key. `tz_offset_minutes` and `tz_name` are presentation hints; storage and queries operate in UTC.

There is no `created_at` / `updated_at` / `recorded_at` field on events. Insertion time is implicit in the rowid. Edit history doesn't exist because events are immutable; corrections are new events.

## Event model

An event represents one measurement act at a single point (or span) in time. It has:

- A type (`event_type`), drawn from the registry (standard or custom).
- Zero or more **channel values** — typed scalar measurements at the leaves of the event type's channel tree. Stored in the EAV `event_channels` table.
- Zero or more **sample streams** — dense numeric series within the event's duration (heart rate over 15 min, ECG µV over 2 min). Stored in `event_samples` as compressed blocks.
- Zero or more **attachments** — large binary payloads (ECG raw, image, PDF). Stored as sidecar files; metadata in `attachments`.
- A device reference (`device_id`) — what hardware produced the measurement.
- An app/version reference (`app_id`) — what software recorded it.
- A source string + idempotency key (`source`, `source_id`) — for deduping repeated imports from the same upstream.
- An optional short freeform `notes` text.
- A possible `superseded_by` — a correction event's pointer to its replacement.
- A soft-delete marker (`deleted_at_ms`).

### Channels are a tree

A channel is a node in a per-event-type tree. Leaves carry typed values (`real`, `int`, `bool`, `text`, `enum`). Group nodes (`is_group=1`) carry no value — only structure.

Examples:

```
blood_pressure                       urine_strip
  systolic   real, mmHg                 glucose       enum(neg/+/++/+++/++++)
  diastolic  real, mmHg                 protein       enum(neg/trace/+/++/+++)
  pulse      real, bpm                  ketones       enum(...)
                                        nitrites      enum(neg/pos)
                                        ph            real
                                        specific_gravity real
                                        leukocytes    enum(...)
                                        blood         enum(...)
                                        urobilinogen  enum(...)
                                        bilirubin     enum(...)

meal
  nutrition (group)
    energy_kcal      real, kcal
    fat (group)
      total          real, g
      saturated      real, g
      unsaturated (group)
        mono         real, g
        poly         real, g
      trans          real, g
    carbohydrates (group)
      total          real, g
      sugars (group)
        total        real, g
        added        real, g
      fiber          real, g
    protein          real, g
    salt             real, g
```

Storage is flat: one row in `event_channels` per *leaf* with a measured value. Group nodes generate no rows. A meal that records only energy and total carbs costs two rows, not the whole tree.

### Standard vs. custom

The registry uses a `namespace` field on `event_types`:

- `std` — the standard catalog, identical across every implementation. Stable channel paths and IDs are part of the format spec.
- `com.<owner>.<name>` — custom user-, app-, or vendor-defined types. Live in the same registry tables, with their own paths and IDs.

When a custom type is promoted to standard, it becomes an alias: existing rows continue to resolve via `type_aliases` / `channel_aliases`, and the background compactor eventually rewrites them to the canonical IDs.

### Immutability

Once written, an event row's values, channels, and samples never change. Three mutations are allowed:

- `superseded_by` is set when a correction event is recorded for this one (one-way pointer).
- `deleted_at_ms` is set on soft delete.
- `device_id`, `app_id`, `source`, `source_id` are immutable.

A correction is itself a new event with its own ULID, `event_type='correction'` (or any type — what matters is the supersedes link), and `metadata` referencing what changed. The original row is preserved in full.

### Privacy is structural, not annotative

Privacy is expressed in the **schema**, not on individual events. Each `event_type` has a `default_sensitivity_class` and each `channel` has a `sensitivity_class` — `general`, `mental_health`, `sexual_health`, `substance_use`, `reproductive`, etc. These are static metadata set when the type or channel is registered.

There are no per-event sensitivity tags. If a user wants part of their data treated more privately than the standard type would imply, the answer is to register (or use) a more sensitive channel or event type and log into that. Concretely: if the user wants to log substance use that one doctor sees and another doesn't, they don't tag specific entries — they log to two distinct types (e.g. `medication_dose` and `medication_dose_private`) whose sensitivity classes differ. Their own queries union both; doctors see only what their grant scopes allow.

This makes the privacy contract the schema itself: looking at a grant's rules and the registry tells you exactly what a grantee sees, with no row-by-row "did this happen to be tagged" branch in the resolver.

## SQL schema

```sql
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;
PRAGMA synchronous=NORMAL;

-- Format / file metadata. One row per key.
CREATE TABLE _meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
-- Required keys at file creation:
--   format_version       e.g. "1.0"
--   user_ulid            the user's wire identity (26-char text or 16-byte hex)
--   deployment_mode      'primary' | 'cache' — see "Deployment modes and sync"
--   created_at_ms        when the file was first opened
--   registry_version     the standard registry version baked in
--   audit_retention_days optional; NULL = forever; otherwise rolling cleanup window
--   cipher_kdf           (if encrypted) KDF parameters

-- Event types (registry)
CREATE TABLE event_types (
  id                        INTEGER PRIMARY KEY,
  namespace                 TEXT NOT NULL,        -- 'std' | 'com.user.x' | 'com.vendor.y'
  name                      TEXT NOT NULL,        -- 'blood_pressure', 'meal', 'pimples_left_arm'
  description               TEXT,
  schema_version            INTEGER NOT NULL DEFAULT 1,
  default_sensitivity_class TEXT NOT NULL DEFAULT 'general',
  UNIQUE (namespace, name)
);

-- Channels (registry; tree-structured per event type)
CREATE TABLE channels (
  id                INTEGER PRIMARY KEY,
  event_type_id     INTEGER NOT NULL REFERENCES event_types(id),
  parent_id         INTEGER REFERENCES channels(id),
  name              TEXT NOT NULL,          -- local segment, e.g. 'saturated'
  path              TEXT NOT NULL,          -- denormalized, e.g. 'nutrition.fat.saturated'
  display_name      TEXT,
  value_type        TEXT NOT NULL,          -- 'real'|'int'|'bool'|'text'|'enum'|'group'
  unit              TEXT,
  enum_values       TEXT,                   -- JSON array, only for value_type='enum'
  is_required       INTEGER NOT NULL DEFAULT 0,
  sensitivity_class TEXT NOT NULL DEFAULT 'general',
  UNIQUE (event_type_id, path)
);

CREATE INDEX idx_channels_parent ON channels (parent_id);

-- Aliases for migrations (old → new). Resolved at read time; compactor rewrites lazily.
CREATE TABLE type_aliases (
  old_namespace TEXT NOT NULL,
  old_name      TEXT NOT NULL,
  new_event_type_id INTEGER NOT NULL REFERENCES event_types(id),
  PRIMARY KEY (old_namespace, old_name)
);

CREATE TABLE channel_aliases (
  event_type_id  INTEGER NOT NULL REFERENCES event_types(id),
  old_path       TEXT NOT NULL,
  new_channel_id INTEGER NOT NULL REFERENCES channels(id),
  PRIMARY KEY (event_type_id, old_path)
);

-- Devices (normalized source of measurements)
CREATE TABLE devices (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  kind          TEXT NOT NULL,          -- 'watch'|'phone'|'cuff'|'cgm'|'scale'|'manual'|'cli'|'mcp'|...
  vendor        TEXT,
  model         TEXT,
  serial_or_id  TEXT,                   -- device-unique identifier when available
  metadata_json TEXT,
  UNIQUE (kind, vendor, model, serial_or_id)
);

-- App / version that recorded the event
CREATE TABLE app_versions (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  app_name  TEXT NOT NULL,              -- 'ohdc-android', 'ohdc-cli', ...
  version   TEXT NOT NULL,              -- semver string
  platform  TEXT,                       -- 'android-14', 'ios-17', 'linux-x86_64', ...
  UNIQUE (app_name, version, platform)
);

-- Events
CREATE TABLE events (
  id                 INTEGER PRIMARY KEY AUTOINCREMENT,
  ulid_random        BLOB NOT NULL,                       -- 10 bytes, 80 random bits
  timestamp_ms       INTEGER NOT NULL,                    -- ms since Unix epoch, UTC
  tz_offset_minutes  INTEGER,                             -- local offset, optional
  tz_name            TEXT,                                -- IANA zone, optional
  duration_ms        INTEGER,                             -- nullable
  event_type_id      INTEGER NOT NULL REFERENCES event_types(id),
  device_id          INTEGER REFERENCES devices(id),
  app_id             INTEGER REFERENCES app_versions(id),
  source             TEXT,                                -- e.g. 'health_connect:com.x.y'
  source_id          TEXT,                                -- idempotency key from the source
  notes              TEXT,                                -- short freeform
  superseded_by      INTEGER REFERENCES events(id),
  origin_peer_id     INTEGER REFERENCES peer_sync(id),    -- NULL = locally minted
  deleted_at_ms      INTEGER
);

CREATE UNIQUE INDEX idx_events_ulid ON events (ulid_random);
CREATE UNIQUE INDEX idx_events_dedup
  ON events (source, source_id) WHERE source_id IS NOT NULL;
CREATE INDEX idx_events_time
  ON events (timestamp_ms DESC) WHERE deleted_at_ms IS NULL;
CREATE INDEX idx_events_type_time
  ON events (event_type_id, timestamp_ms DESC) WHERE deleted_at_ms IS NULL;

-- Channel values (EAV, sparse)
CREATE TABLE event_channels (
  event_id    INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
  channel_id  INTEGER NOT NULL REFERENCES channels(id),
  value_real  REAL,
  value_int   INTEGER,
  value_text  TEXT,
  value_enum  INTEGER,             -- ordinal index into channels.enum_values
  PRIMARY KEY (event_id, channel_id)
);

CREATE INDEX idx_channels_by_channel
  ON event_channels (channel_id, event_id);

-- Dense numeric streams (HR, glucose, ECG)
CREATE TABLE event_samples (
  event_id      INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
  channel_id    INTEGER NOT NULL REFERENCES channels(id),
  block_index   INTEGER NOT NULL,        -- 0-based, dense within (event,channel)
  t0_ms         INTEGER NOT NULL,        -- absolute start of this block
  t1_ms         INTEGER NOT NULL,        -- absolute end of this block
  sample_count  INTEGER NOT NULL,
  encoding      INTEGER NOT NULL,        -- codec ID (see "sample blocks")
  data          BLOB NOT NULL,           -- compressed (t,v) pairs
  PRIMARY KEY (event_id, channel_id, block_index)
);

CREATE INDEX idx_samples_time
  ON event_samples (channel_id, t0_ms);

-- Attachments (metadata only; payload lives as sidecar file)
CREATE TABLE attachments (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  ulid_random BLOB NOT NULL UNIQUE,
  event_id    INTEGER NOT NULL REFERENCES events(id) ON DELETE CASCADE,
  sha256      BLOB NOT NULL,             -- 32 bytes; addresses the sidecar file
  byte_size   INTEGER NOT NULL,
  mime_type   TEXT,
  filename    TEXT,
  encrypted   INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX idx_attachments_event ON attachments (event_id);

-- Grants (per-user). Policy fields are typed columns; scope is in rule tables below.
-- A grant is the universal access primitive: read grants for third-party readers,
-- write grants for clinical workflows (with optional approval queue), and device
-- tokens which are grants with kind='device' and write-only-no-expiry policy.
CREATE TABLE grants (
  id                         INTEGER PRIMARY KEY AUTOINCREMENT,
  ulid_random                BLOB NOT NULL UNIQUE,
  grantee_label              TEXT NOT NULL,           -- e.g. "Dr. Smith — primary care"
  grantee_kind               TEXT NOT NULL,           -- 'human'|'app'|'service'|'emergency'|'device'|'self'|'delegate'
  grantee_ulid               BLOB,                    -- if grantee has its own OHD identity
  created_at_ms              INTEGER NOT NULL,
  expires_at_ms              INTEGER,
  revoked_at_ms              INTEGER,
  purpose                    TEXT,
  -- Read scope behaviour
  default_action             TEXT NOT NULL DEFAULT 'deny',  -- 'allow' (denylist) | 'deny' (allowlist) for reads
  aggregation_only           INTEGER NOT NULL DEFAULT 0,
  strip_notes                INTEGER NOT NULL DEFAULT 1,
  require_approval_per_query INTEGER NOT NULL DEFAULT 0,
  -- Write scope behaviour
  approval_mode              TEXT NOT NULL DEFAULT 'always',  -- 'always'|'auto_for_event_types'|'never_required'
  -- General policy
  notify_on_access           INTEGER NOT NULL DEFAULT 0,
  max_queries_per_day        INTEGER,
  max_queries_per_hour       INTEGER,
  rolling_window_days        INTEGER                  -- if set, only events in last N days visible
);

CREATE INDEX idx_grants_active  ON grants (revoked_at_ms, expires_at_ms);
CREATE INDEX idx_grants_grantee ON grants (grantee_ulid) WHERE grantee_ulid IS NOT NULL;
CREATE INDEX idx_grants_kind    ON grants (grantee_kind);

-- Grant read rules: by event type
CREATE TABLE grant_event_type_rules (
  grant_id      INTEGER NOT NULL REFERENCES grants(id) ON DELETE CASCADE,
  event_type_id INTEGER NOT NULL REFERENCES event_types(id),
  effect        TEXT NOT NULL,             -- 'allow' | 'deny'
  PRIMARY KEY (grant_id, event_type_id)
);

-- Grant read rules: by individual channel (more granular than type)
CREATE TABLE grant_channel_rules (
  grant_id    INTEGER NOT NULL REFERENCES grants(id) ON DELETE CASCADE,
  channel_id  INTEGER NOT NULL REFERENCES channels(id),
  effect      TEXT NOT NULL,
  PRIMARY KEY (grant_id, channel_id)
);

-- Grant read rules: by sensitivity class
CREATE TABLE grant_sensitivity_rules (
  grant_id          INTEGER NOT NULL REFERENCES grants(id) ON DELETE CASCADE,
  sensitivity_class TEXT NOT NULL,
  effect            TEXT NOT NULL,
  PRIMARY KEY (grant_id, sensitivity_class)
);

-- Grant write rules: which event types the grantee can submit. Default empty (read-only grant).
CREATE TABLE grant_write_event_type_rules (
  grant_id      INTEGER NOT NULL REFERENCES grants(id) ON DELETE CASCADE,
  event_type_id INTEGER NOT NULL REFERENCES event_types(id),
  effect        TEXT NOT NULL,             -- 'allow' | 'deny'
  PRIMARY KEY (grant_id, event_type_id)
);

-- Grant write rules: which event types auto-commit (skip approval queue) when
-- approval_mode='auto_for_event_types'. Empty for 'always' or 'never_required'.
CREATE TABLE grant_auto_approve_event_types (
  grant_id      INTEGER NOT NULL REFERENCES grants(id) ON DELETE CASCADE,
  event_type_id INTEGER NOT NULL REFERENCES event_types(id),
  PRIMARY KEY (grant_id, event_type_id)
);

-- Grant rules: absolute time window (one row per grant; rolling_window_days lives on the grant)
CREATE TABLE grant_time_windows (
  grant_id INTEGER PRIMARY KEY REFERENCES grants(id) ON DELETE CASCADE,
  from_ms  INTEGER,
  to_ms    INTEGER
);

-- Pending events: grant-submitted writes awaiting user review. Promoted to
-- the events table on approval, retained as a record on rejection/expiry.
CREATE TABLE pending_events (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  ulid_random         BLOB NOT NULL UNIQUE,
  submitted_at_ms     INTEGER NOT NULL,
  submitting_grant_id INTEGER NOT NULL REFERENCES grants(id),
  payload_json        TEXT NOT NULL,         -- canonical event JSON for review
  status              TEXT NOT NULL,         -- 'pending'|'approved'|'rejected'|'expired'
  reviewed_at_ms      INTEGER,
  rejection_reason    TEXT,
  expires_at_ms       INTEGER NOT NULL,
  approved_event_id   INTEGER REFERENCES events(id)  -- set when approved
);

CREATE INDEX idx_pending_status ON pending_events (status, submitted_at_ms);
CREATE INDEX idx_pending_grant  ON pending_events (submitting_grant_id, status);

-- Per-user audit log
CREATE TABLE audit_log (
  id                INTEGER PRIMARY KEY AUTOINCREMENT,
  ts_ms             INTEGER NOT NULL,
  actor_type        TEXT NOT NULL,           -- 'self'|'grant'|'system' (device tokens recorded as grant with grants.kind='device')
  grant_id          INTEGER REFERENCES grants(id),  -- null when actor_type='self' or 'system'
  action            TEXT NOT NULL,           -- 'read'|'write'|'delete'|'export'|'import'|'grant_*'|'login'|'config'
  query_kind        TEXT,                    -- for reads: 'list_events'|'aggregate'|'sample_read'|'export'
  query_params_json TEXT,                    -- canonicalized request payload
  rows_returned     INTEGER,
  rows_filtered     INTEGER,                 -- matched rows stripped silently by grant rules
  result            TEXT NOT NULL,           -- 'success'|'partial'|'rejected'|'error'
  reason            TEXT,                    -- when rejected/partial/error
  caller_ip         TEXT,
  caller_ua         TEXT
);

CREATE INDEX idx_audit_time   ON audit_log (ts_ms DESC);
CREATE INDEX idx_audit_grant  ON audit_log (grant_id, ts_ms DESC);
CREATE INDEX idx_audit_action ON audit_log (action, ts_ms DESC);

-- Peer sync state: one row per peer this file has ever synced with.
-- See "Deployment modes and sync" for the protocol semantics.
CREATE TABLE peer_sync (
  id                       INTEGER PRIMARY KEY AUTOINCREMENT,
  peer_label               TEXT NOT NULL UNIQUE,    -- e.g. 'server:ohd.example.org', 'phone:device-1234'
  peer_kind                TEXT NOT NULL,           -- 'server' | 'phone' | 'desktop' | 'mirror'
  peer_ulid                BLOB,                    -- if peer has its own OHD identity
  last_outbound_rowid      INTEGER NOT NULL DEFAULT 0,   -- our local rowid the peer has acked
  last_inbound_peer_rowid  INTEGER NOT NULL DEFAULT 0,   -- peer's rowid we have consumed up to
  last_sync_started_ms     INTEGER,
  last_sync_ok_ms          INTEGER,
  last_status              TEXT                     -- 'ok' | 'error' | 'stalled'
);

CREATE INDEX idx_events_origin
  ON events (origin_peer_id, id) WHERE origin_peer_id IS NULL;
```

JSON appears only in four places, all of which are deliberate:
- `channels.enum_values` — small static enum list, set once per channel definition.
- `audit_log.query_params_json` — canonicalized request payload for forensics/replay; never a query target.
- `devices.metadata_json` — opaque vendor-specific device metadata; rarely accessed.
- `pending_events.payload_json` — the full event submission held for user review; consumed once when promoted to `events` (or rejected/expired).

Hot-path event data, grant rules, and access control all live in typed columns. No `data JSONB` anywhere; rule evaluation is pure SQL.

## Sample blocks

Dense numeric streams are stored as windowed compressed blocks rather than per-sample rows. Default window: 15 minutes (~900 seconds) per block. Configurable per channel.

Each block holds `(t_offset_ms, value)` pairs where `t_offset_ms` is relative to `t0_ms`. The on-disk encoding is one of:

- **Encoding `1`** — *Delta-zigzag-varint timestamps + float32 values, zstd-compressed.*
  Layout (uncompressed): `varint(sample_count) || zigzag_varint(dt_0) || float32(v_0) || zigzag_varint(dt_1 - dt_0) || float32(v_1) || …`
  Compressed with zstd level 3.
- **Encoding `2`** — *Delta-zigzag-varint timestamps + int16 quantized values + scale.*
  Layout (uncompressed): `varint(sample_count) || float32(scale) || float32(offset) || zigzag_varint(dt_0) || int16(q_0) || …`
  Useful for integer-quantized streams (HR bpm, step counts) at ~half the size of encoding 1.
  Decoded value: `q_i * scale + offset`.

Implementations MUST support encoding 1 for read. Encoding 2 is strongly recommended. Future encodings may be added; readers ignore unknown encoding IDs (returning a typed error).

Encoding determinism: given the same input samples and parameters, encoders MUST produce byte-identical blocks. This is checked by the conformance test suite.

## Channel registry

The standard registry (`namespace='std'`) is part of the format specification. It ships as a versioned JSON catalog (`registry/v1.json`) that every implementation embeds. On file creation, the implementation populates the file's `event_types` and `channels` tables from the catalog. Stable IDs in the catalog ensure cross-implementation consistency.

When the standard registry version is bumped:

- The embedded catalog gets new entries (additive).
- A small migration on file open adds any missing standard rows.
- Existing custom rows (in `com.*` namespaces) are untouched.
- `_meta.registry_version` is updated.

Custom event types and channels are added by the user, the app, or third-party connectors:

```python
store.registry.add_event_type(namespace="com.jakub.skin", name="left_arm")
store.registry.add_channel(
    event_type=("com.jakub.skin", "left_arm"),
    path="pimple_count",
    value_type="int",
)
```

These are written to the same `event_types` / `channels` tables and round-trip through export/import.

**Validation on write.** When `put_events` is called:

- Each event's type must resolve to a known `event_type_id` (directly or via `type_aliases`).
- Each channel value must resolve to a known leaf `channel_id` for that type (directly or via `channel_aliases`).
- The value must match the channel's `value_type`.
- For `enum` channels, the value must be one of `enum_values`.
- Required channels must be present.

Violations are rejected at the API boundary, never silently coerced.

## Migrations

Schema changes happen in three forms, none of which require a whole-file rewrite:

1. **Adding a standard channel/type.** Pure additive. New rows reference new IDs; old events are unaffected.
2. **Renaming/restructuring** (`com.user.x` → `std`, or `nutrition.fat` reorganized). Adds entries to `type_aliases` / `channel_aliases`. Reads transparently resolve old paths to new IDs. Writers may continue to write old or new paths; new writes get canonical IDs.
3. **Compaction** (background, optional). A worker scans rows whose `event_type_id` or `channel_id` has an alias pointing forward, rewrites the FK, and removes resolved alias entries. Runs in small batches; never holds long locks. Idempotent: safe to interrupt and resume.

Aliases are append-only; the compactor's only mutation is to rewrite event/channel rows to canonical IDs.

## Privacy and access control

Every external operation is authenticated under one of three auth profiles. What the operation can do is bounded by the profile's scope.

### The three auth profiles

| Profile | Used by | Read scope | Write scope | Token resolution |
|---|---|---|---|---|
| **Self-session** | The user themselves (OIDC-authenticated) | Full on own data | Full | OIDC subject → `_meta.user_ulid` match |
| **Grant token** | Third parties (clinical apps, family / delegate, researchers) | Bounded by the grant's structured rules | Bounded by the grant's write rules; optional approval queue | Token → `grants` row by `ulid_random` |
| **Device token** | Sensor / lab / pharmacy / EHR integrations | None (write-only) | Append events attributed by `device_id` | Token → `grants` row with `kind='device'` |

A device token is structurally a grant — a row in `grants` with `kind='device'`, write-only scope, no expiry, attributed by `device_id`. The auth handler treats device tokens uniformly with other grants; the cheaper damage cap (write-only, write-attributed) is what makes them issuable freely to integrations.

### The grant

A grant identifies a grantee (a doctor, an app, a researcher, an emergency contact, a sensor) and bounds what they can do. Its row in `grants` carries:

- A user-given `grantee_label` and a classifying `grantee_kind` (`human` / `app` / `service` / `emergency` / `device` / `delegate`). The label is whatever the user types ("Dr. Smith — primary care"); the kind classifies the grantee. No PII is required.
- An optional `grantee_ulid` if the grantee has their own OHD identity. Pseudonymous grants without a `grantee_ulid` are valid — token possession alone authorizes.
- **Read policy**: `default_action` (allow = denylist, deny = allowlist), `aggregation_only`, `strip_notes`, `require_approval_per_query`, `rolling_window_days`.
- **Write policy**: `approval_mode` — `always` (every write queues for review), `auto_for_event_types` (pre-authorized types auto-commit; others queue), `never_required` (all writes auto-commit; used for trusted long-term grants and emergency / break-glass).
- **Lifecycle**: `expires_at_ms`, `revoked_at_ms`, `notify_on_access`, `max_queries_per_day`/`hour`.

### The rules

Read rules refine what a grant can see:

| Surface | Granularity |
|---|---|
| `grant_event_type_rules` | by event type (e.g. `medication_dose`) |
| `grant_channel_rules` | by individual channel (e.g. `meal.nutrition.fat.saturated`) |
| `grant_sensitivity_rules` | by sensitivity class (e.g. `mental_health`, `sexual_health`) |
| `grant_time_windows` | absolute `from_ms`/`to_ms` window |
| `grants.rolling_window_days` | rolling time bound (last N days) |

Write rules refine what a grant can submit:

| Surface | Granularity |
|---|---|
| `grant_write_event_type_rules` | by event type. Default: empty (read-only grant). |
| `grant_auto_approve_event_types` | which event types auto-commit when `approval_mode='auto_for_event_types'`. |

Each rule has an `effect` of `allow` or `deny`. Together with the grant's `default_action` (read) and `approval_mode` (write), they fully determine what the grant can read, what it can submit, and whether submissions queue or auto-commit. There are no per-event privacy annotations to consult — sensitivity comes from the schema.

### Query resolution

When a query references a "logical" name — `glucose`, `meal.nutrition.fat.saturated`, or an event type like `medication_dose` — the library resolves it to a concrete set of channel/type IDs in three steps:

1. **Lookup.** Match the logical name against the registry, including `channel_aliases` and `type_aliases`. A single logical name may map to multiple IDs:
   - When the user has registered private/public variants of the same concept (`medication_dose` and `medication_dose_personal`), both contribute to the user's own self-query.
   - When migrations are mid-flight, old + new IDs are both active via aliases.
2. **Permission intersection** (grant queries only). The resolved set is intersected with the channels and types the grant allows under the resolution algorithm below. Self-queries skip this step entirely.
3. **SQL.** The final ID set goes directly into `WHERE channel_id IN (…)` / `WHERE event_type_id IN (…)`.

This is the only place where channel/type plurality affects the query path, and it falls out of the same machinery that powers aliases — no special-case code. Runtime cost is negligible: the registry holds at most low thousands of entries per file, all in memory, indexed by name and path; a full self-query against a 1M-event file is the same indexed scan whether one or three IDs end up in the IN-set.

### The resolution algorithm

For each candidate event being considered for return:

1. Start with the grant's `default_action`.
2. **Time eligibility** — if outside `grant_time_windows.[from_ms, to_ms]` or older than `rolling_window_days`, deny.
3. **Sensitivity class** — examine the event's `event_type.default_sensitivity_class` and the `sensitivity_class` of each present channel. Apply matching `grant_sensitivity_rules`.
4. **Event type** — apply matching row in `grant_event_type_rules`.
5. **Resolve.** If allowed, continue to channel filtering; if denied, the event is omitted entirely from the result set and counted into `rows_filtered`.

If allowed, channel-level filtering then determines which channels are returned for the event:

6. For each channel on the event, apply `grant_channel_rules` and the channel's `sensitivity_class` against `grant_sensitivity_rules`. Hidden channels are stripped from the row.
7. If `aggregation_only=1`, raw events are not returned at all — only aggregate queries succeed for this grant.
8. If `strip_notes=1`, the `notes` column is replaced with NULL on returned rows.

**Deny wins on conflict.** If an event matches both an allow rule and a deny rule (e.g. allowlisted by type but denylisted by sensitivity class), deny wins. This makes "I gave Dr. Skin access to skin events but later globally flagged `sexual_health` as hidden" do the safe thing without re-checking every grant.

### Write-with-approval

Grant tokens with write scope can be configured to route submissions through an approval queue. The grant's `approval_mode` determines per-submission behaviour:

- `always` — every submission goes to `pending_events`. The user reviews each before it commits to canonical storage.
- `auto_for_event_types` — submissions whose `event_type_id` is in `grant_auto_approve_event_types` commit immediately to `events`; others queue. Used for established relationships where routine writes (e.g. `lab_result`, `clinical_note`) are pre-authorized while high-stakes writes (e.g. `prescription`) still get reviewed.
- `never_required` — all submissions commit immediately. Used for trusted long-term grants and emergency / break-glass cases where queueing would be malpractice.

**The pending-event flow:**

1. The grantee submits an event via `put_events` against their grant token.
2. Storage validates the event (registry check) and authorizes the write (against `grant_write_event_type_rules`).
3. If the grant's `approval_mode` requires approval for this event type, storage allocates a ULID and writes the event into `pending_events` with `status='pending'`. Otherwise commits to `events` directly.
4. For pending events, the user gets a notification on their personal app.
5. User reviews via OHD Connect:
   - **Approve**: storage moves the event into `events` with the same ULID, sets `pending_events.status='approved'` and `approved_event_id`. Audit log records both the original submission and the approval.
   - **Reject**: `pending_events.status='rejected'` with an optional reason. Audit log records both the submission and the rejection.
6. Auto-expire after `expires_at_ms`. Status flips to `expired`.

The grantee always sees the pending status of their submission (via `list_pending` against their own token, scoped to their grant's submissions). The grantee never sees the user's review reasoning. The user always sees what was submitted, even rejected/expired entries (durable record).

ULID identity is preserved across the pending → committed transition: the ULID minted at submission stays attached to the canonical event when approved. Cross-event references (e.g. `superseded_by`, `part_of_event_id`) work consistently before and after promotion.

### Audit

Every query — accepted, partial, or rejected — produces an `audit_log` row:

- `grant_id` references the grant the query came under (NULL for `self` and `system`).
- `query_kind` and `query_params_json` capture what was asked.
- `rows_returned` and `rows_filtered` capture what was given vs. silently stripped. The grantee never sees that something was filtered; the user always does.
- `result` distinguishes `success`, `partial` (some rows or channels stripped), `rejected` (request out of scope), `error`.

Listing "what has Dr. Smith queried this month" is a single indexed scan: `audit_log WHERE grant_id=? AND ts_ms BETWEEN ?`.

Retention is configurable via `_meta.audit_retention_days`. Default is `NULL` (forever). Setting a finite value enables a background cleanup pass that drops `audit_log` rows older than the window. Account-lifecycle events that must outlive user-file deletion live in the deployment's separate system-level DB (see "Deployment modes and sync"), not here.

### Revocation semantics

Grant revocation is a **synchronous RPC**, not a sync-deferred event:

- **Local-primary mode**: revocation runs locally against the user's own file, sets `revoked_at_ms`, and takes effect immediately. Subsequent grant lookups return revoked.
- **Cache mode**: revocation is an RPC from cache to primary. The primary's file is the source of truth for grants. The call either succeeds (primary acknowledges, sets `revoked_at_ms`, replies OK) or fails (network down, primary unreachable, etc.). No queueing. The user sees an error and retries when connectivity returns.
- **Sync stream is not used** for revocations. Sync replays event creation/correction/deletion; grant lifecycle changes are out-of-band RPCs because their latency requirements are different (panic-revoke must be immediate or fail clearly, never silently buffered for "next sync").

Once the primary has marked a grant as revoked, the next regular sync pulls the updated `grants` row to the cache as part of normal replication. But the *revocation effect* — that subsequent grantee queries are denied — is in force from the moment the primary commits, not from the moment the cache sees the update.

### Grant token format

The format of the bearer token a grantee presents (opaque random, JWT, signed envelope, etc.) is an API-layer concern. The storage format requires only that the token resolves uniquely to a `grants.id` (or equivalently, to a `grants.ulid_random`). Implementations may pick any format that satisfies this; nothing in the on-disk schema depends on the choice.

### Inspecting from the user's side

Because rules are structured, the user (or their personal dashboard) can ask:

- *What event types can Dr. Smith see?* → `grant_event_type_rules ⋈ event_types`.
- *What can Dr. Smith write?* → `grant_write_event_type_rules ⋈ event_types`, plus the grant's `approval_mode`.
- *What's pending review from Dr. Smith?* → `pending_events WHERE submitting_grant_id=? AND status='pending'`.
- *Has anything Dr. Smith requested been silently filtered?* → `audit_log WHERE grant_id=? AND rows_filtered > 0`.
- *Which grants can see my mental_health data?* → `grant_sensitivity_rules WHERE sensitivity_class='mental_health' AND effect='allow'`, plus default-allow grants without an offsetting deny.
- *Which devices have written to my storage in the last week?* → `events ⋈ devices` filtered by recent timestamp; or `grants WHERE kind='device' AND last_used > ?`.
- *Show me a preview of what Dr. Smith would see for a glucose query right now.* → run the resolver against the current data with that grant's rules.

All single-table or small-join queries because no JSON unpack is involved.

### Emergency access

The "break-glass" emergency grant from `privacy-access.md` is just a normal grant with a curated rule set. The library ships a helper:

```python
store.grants.create_emergency(
    label="Emergency responders",
    expires_at=None,                # long-lived
    notify_on_access=True,          # always notify
    allowed_types=["allergy", "medication_prescribed", "diagnosis", "vaccination",
                   "blood_type", "advance_directive"],
    denied_sensitivity=["mental_health", "substance_use", "sexual_health", "reproductive"],
)
```

The user reviews the resulting rules and can adjust before issuing the token (typically encoded as a QR on a wristband or lock screen).

## Deployment modes and sync

A given file plays one of two roles, set at file creation in `_meta.deployment_mode`:

- **`primary`** — canonical for the user. Accepts writes, serves external grant queries, runs the full grant resolution algorithm. The user's source of truth.
- **`cache`** — mirrors a remote `primary`. Accepts local writes which are queued and flushed to the primary. Read queries serve from local data; if data is missing or stale, the runtime can fall through to the primary (subject to network availability). Cannot serve external grant queries — those go to the primary.

  **Cache mode never auto-evicts.** Health data is too costly to silently drop. The library surfaces storage pressure as runtime events (warning thresholds, critical thresholds, write-failure on physical full); the application decides what to do. If the device truly runs out of space, writes fail loudly so the user can act — relocate the file, free other space, or explicitly authorize a one-shot purge of the oldest non-locally-originated rows. Local-origin events that have not yet been acked by the primary (`origin_peer_id IS NULL AND id > peer_sync.last_outbound_rowid`) are **never** evictable, including under user-confirmed purge — they are the only authoritative copy until sync completes.

The user picks a mode at first-app-launch:

- **Local-primary deployment** — phone is `primary`; an optional server is a passive `mirror` or absent. Phone runs everything locally, including grant resolution for doctors who reach the phone (via relay, dynamic DNS, or LAN). The user's data stays on their device.
- **Remote-primary deployment** — server is `primary`; phone is `cache`. Phone collects measurements, flushes them to the server in the background, and serves user-side reads from local cache. Doctors query the server. Selectable across multiple SaaS providers, self-hosted servers, or hospital deployments.

Both modes share the same storage format and library. The `deployment_mode` flag determines runtime behavior. Both must be implemented for public launch — neither is a shortcut.

### Sync protocol (logical model)

Sync between two files is **bidirectional event-log replay** with idempotency from ULID identity:

- Each side maintains a `peer_sync` row per peer it has ever synced with — `last_outbound_rowid` (our local rowid the peer has acknowledged) and `last_inbound_peer_rowid` (the peer's local rowid we have consumed up to), plus last-sync timestamps and status.
- An **outbound** sync sends events satisfying `events.id > last_outbound_rowid AND origin_peer_id IS NULL` (local-origin events the peer hasn't seen). The peer inserts each, deduping on `ulid_random`. The peer's ack updates our `last_outbound_rowid`.
- An **inbound** sync requests "events with peer-rowid > `last_inbound_peer_rowid`". The peer ships each event tagged with its local rowid; we insert each with `origin_peer_id` set, dedupe on `ulid_random`, and advance our watermark.
- Watermarks are based on **insertion order (rowid)**, not on event timestamp or ULID time. This keeps sync correct in the presence of backfilled events, including pre-1970 events whose ULID time prefix is clamped to 0.
- Corrections (`superseded_by` set), soft-deletes (`deleted_at_ms` set), grants, and grant rules sync as ordinary rows. Immutability + ULID identity means there are no conflicts to merge — sync is "send me what I haven't seen," nothing else.
- Audit log entries do **not** sync by default. Each instance audits its own access. When a remote-origin event is imported into a file, a local audit row is written tagged `actor_type='system'` and `action='import'` for traceability.

The wire protocol for sync (framing, batching, compression, retry, authentication) is the OHD-Core API's concern, not the storage spec's. The storage primitives needed (`peer_sync`, `origin_peer_id`, ULID dedup) live here.

### Last-connector-seen tracking

For SaaS deployments running `primary` for users whose phones run `cache`, false alerts are a real risk: "user missed medication" should not fire when the gap is just sync lag from the phone being offline. The deployment's **system-level DB** (separate from any user file) tracks per-user, per-cache `last_seen_at_ms`. Alert engines check this before firing time-sensitive notifications.

This is the same system-level DB that holds account-lifecycle audit (file created, file deleted, key rotated, OIDC events, abuse signals) — the rule being: *if a row only makes sense given the user's data, it's per-user; if it must survive when the user is forgotten, it's system-level.* Retention policies on the system DB are a deployment concern, separate from the per-user `audit_retention_days` config.

## Encryption

**At rest.** SQLCipher 4 with page-level AES-256. The per-user key derives from a user-held secret (passphrase, biometric-unlocked keystore item, hardware token) plus a per-file salt stored in `_meta.cipher_kdf`. KDF: PBKDF2-SHA512, 256k iterations (SQLCipher 4 default).

*Future revisit*: SQLCipher 5 (when released) is expected to support Argon2id, which has stronger memory-hardness against GPU/ASIC brute-force. Migrate to Argon2id at that point, with a one-time KDF re-derivation pass per file (transparent to callers because it doesn't change the data, only the key-derivation parameters in `_meta.cipher_kdf`).

**Sidecar blobs.** Encrypted with the same per-user key using libsodium `crypto_secretstream` (or equivalent AEAD). Each blob is independently decryptable; metadata in `attachments.sha256` is the address; integrity is checked on read.

**In transit.** TLS 1.3 (handled by the transport layer, not storage).

**End-to-end channel encryption.** Channel-level encryption for sensitive fields (mental health, substance use, sexual health) using a separate user-held key, so the storage operator cannot read those fields even at the engine level. The format reserves room for it but the bit-level details (key wrapping, grant-side ciphertext semantics) are not yet specified. Listed in "Open design items" below.

## Concurrency

SQLite WAL mode allows one writer + many readers per file. The library enforces this:

- A `Store` instance owns at most one writer connection per file.
- Read connections are pooled (default 4).
- The compactor takes the writer lock briefly per batch; never longer than 100 ms.
- Cross-process access to the same file is *not supported*. Open it from one process at a time; if you need multi-process, use the API server in front.

## Wire format and transport

The storage library exposes a transport-agnostic API:

```python
store.put_events(events: list[Event]) -> list[Ulid]
store.query_events(filter: Query, cursor: Cursor | None = None) -> Page[Event]
store.aggregate(channel: str, frm: int, to: int, op: str, bucket: str) -> list[Bucket]
store.attach(event_id: Ulid, blob: bytes, mime: str) -> AttachmentRef
store.export(sink: BinaryIO) -> ExportManifest
store.import_(source: BinaryIO) -> ImportReport
```

Transport choice is the API server's concern, not the storage library's. Recommended:

- **HTTP/3 (QUIC) preferred, HTTP/2 over TCP fallback**. Caddy 2.6+ handles negotiation. Connectors use platform-native HTTP/3 stacks (URLSession on iOS, Cronet on Android).
- **Per-event idempotency** via `(source, source_id)` and ULID. Retries are safe.
- **Batch writes** are first-class: `put_events([...])` is one transaction.
- **Blobs** travel over HTTPS as separate uploads (chunked if needed); the event references the resulting SHA-256.

Custom UDP / message-bus protocols are possible but unnecessary for OHD's projected scale. Revisit only if QUIC overhead becomes a measured bottleneck.

## Implementation

A single Rust core implements the format and the OHDC interface (with its three auth profiles: self-session, grant token, device token). Bindings for Kotlin (Android) and Swift (iOS) are generated via `uniffi`; a `PyO3` binding exposes the same core to Python for tooling and the conformance test harness. Same on-disk bytes everywhere; a file written by Android can be opened on a Linux server unchanged.

The engine is SQLite + SQLCipher. Concurrency is single-writer + many-readers (WAL mode) per file. Cross-process access to the same file is not supported — one process owns the writer.

A conformance corpus (a known input event sequence + expected query outputs) is part of the format spec, so any future re-implementation can be checked for byte and semantic compatibility.

## Versioning

`_meta.format_version` is set on file creation and never decreased. Format versions follow semver:

- **Patch**: documentation/clarification only.
- **Minor**: additive (new tables, new columns with defaults, new sample-block encodings, new standard registry entries). Old readers can open new files; some new fields are invisible to them.
- **Major**: breaking. Old readers refuse the file; an explicit migration tool produces a new file at the new version.

Compatibility rules:

- A library MUST refuse to open a file with a higher major version than it supports.
- A library MUST open files with equal-or-lower minor versions, applying any forward-compat shims.
- A migration tool always exists for the previous major version. No file is ever stranded.

## Open design items

These have a place reserved in the format but no bit-level specification yet:

1. **End-to-end channel encryption.** Channel-level ciphertext for the most sensitive sensitivity classes (mental_health, sexual_health, substance_use, reproductive), wrapped in a key the storage operator does not hold. The grants table needs per-grant key-wrap material to share the right plaintext with the right grantee. Open: column shape, KDF parameters, the grant-handoff handshake.
2. **Standard registry governance.** The catalog ships with the format. The process by which new entries get added (PR + version bump, registry council, etc.) is not yet defined; it'll be spec'd when the project has more than one contributor.

---

## Cross-references

- Conceptual event vocabulary: [`data-model.md`](data-model.md)
- Privacy, grants, audit semantics: [`privacy-access.md`](privacy-access.md)
- Deployment topology: [`deployment.md`](deployment.md)
- Core service that uses this storage: [`../components/storage.md`](../components/storage.md)
- OHDC protocol and the OHD Connect personal app: [`../components/connect.md`](../components/connect.md)
- OHD Care reference clinical app: [`../components/care.md`](../components/care.md)
- OHD Relay bridging service: [`../components/relay.md`](../components/relay.md)
