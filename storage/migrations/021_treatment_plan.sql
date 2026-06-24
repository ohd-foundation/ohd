-- Treatment plans, tracked items & the "life" case. (Migration 021.)
--
-- Builds on 020. Adds the model behind the on-device tracking redesign
-- (see plan deep-dancing-teacup.md "Tracked items, treatment plans & the
-- 'life' case"):
--
--   A *tracked item* (a medication regimen or a measurement watch) carries
--   `case_id` (a specific clinical episode, or absent = the implicit global
--   "life" case), an optional loose `schedule` (a 5-field cron expr OR
--   `anchor:<name>` like `anchor:lunch` — stored & displayed, no engine
--   yet), `on_hand` (bool — "I have this", inventory presence) and `quick`
--   (bool — surface as a one-tap shortcut).
--
--   measurement.watch_started / _stopped — a measurement the user is
--   tracking on a cadence ("watch my temperature daily"). Active =
--   started-without-stopped, mirroring medication regimens. Readings are
--   ordinary measurement.* events; the watch only declares intent + schedule.
--
--   profile.condition gains an optional `case_id` so a diagnosis can be
--   episodic (tied to a case) vs chronic/long-term (no case → shown in the
--   health profile). "Becomes chronic" = recorded without a case.
--
-- Idempotent — every INSERT is OR IGNORE. Sensitivity all `general`
-- (rationale in 020's header).

-- ===========================================================================
-- medication.regimen_started — add schedule / on_hand / quick.
-- ===========================================================================
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'schedule', 'schedule', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='regimen_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'on_hand', 'on_hand', 'bool', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='regimen_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'quick', 'quick', 'bool', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='regimen_started';

-- ===========================================================================
-- measurement.watch_started — a tracked measurement (metric + cadence).
-- ===========================================================================
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('measurement', 'watch_started', 'Start tracking a measurement on a cadence', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'watch_id', 'watch_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='watch_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'metric', 'metric', 'text', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='watch_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'label', 'label', 'text', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='watch_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'schedule', 'schedule', 'text', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='watch_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'on_hand', 'on_hand', 'bool', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='watch_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'quick', 'quick', 'bool', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='watch_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'case_id', 'case_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='watch_started';

-- ===========================================================================
-- measurement.watch_stopped
-- ===========================================================================
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('measurement', 'watch_stopped', 'Stop tracking a measurement', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'watch_id', 'watch_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='watch_stopped';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'reason', 'reason', 'text', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='watch_stopped';

-- ===========================================================================
-- profile.condition — add optional case_id (episodic vs chronic).
-- ===========================================================================
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'case_id', 'case_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='condition';

-- ===========================================================================
-- Common measurement.* reading types — add optional case_id so a single
-- reading can attach to a case ("logged into the current episode"). No row
-- matches for a type that isn't registered yet, so this is safe + sparse.
-- ===========================================================================
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'case_id', 'case_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='measurement'
  AND name IN ('blood_pressure', 'glucose', 'body_weight', 'body_temperature', 'heart_rate', 'spo2');
