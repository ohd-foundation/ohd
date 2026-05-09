-- Case lifecycle marker event types.
--
-- Per `spec/storage-format.md` "Case lifecycle markers": `std.case_started` /
-- `std.case_closed` / `std.case_reopened` / `std.case_handoff` are written into
-- `events` for the patient's timeline display. They reference the case by
-- ULID via the `case_ref_ulid` channel (text). These events are NOT how a
-- case knows its own membership — that's still `case_filters`. The lifecycle
-- events are purely chronological-view markers.
--
-- Idempotent — uses INSERT OR IGNORE everywhere.

INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'case_started', 'Case lifecycle marker: case opened', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'case_ref_ulid', 'case_ref_ulid', 'text', NULL, 'general'
  FROM event_types WHERE namespace='std' AND name='case_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'case_type', 'case_type', 'text', NULL, 'general'
  FROM event_types WHERE namespace='std' AND name='case_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'case_label', 'case_label', 'text', NULL, 'general'
  FROM event_types WHERE namespace='std' AND name='case_started';

INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'case_closed', 'Case lifecycle marker: case closed', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'case_ref_ulid', 'case_ref_ulid', 'text', NULL, 'general'
  FROM event_types WHERE namespace='std' AND name='case_closed';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'reason', 'reason', 'text', NULL, 'general'
  FROM event_types WHERE namespace='std' AND name='case_closed';

INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'case_reopened', 'Case lifecycle marker: case reopened', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'case_ref_ulid', 'case_ref_ulid', 'text', NULL, 'general'
  FROM event_types WHERE namespace='std' AND name='case_reopened';

INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'case_handoff', 'Case lifecycle marker: handoff to a successor case', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'case_ref_ulid', 'case_ref_ulid', 'text', NULL, 'general'
  FROM event_types WHERE namespace='std' AND name='case_handoff';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'successor_case_ulid', 'successor_case_ulid', 'text', NULL, 'general'
  FROM event_types WHERE namespace='std' AND name='case_handoff';
