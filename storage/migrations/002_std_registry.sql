-- Standard registry seed.
--
-- Distilled from `spec/data-model.md`. Covers the v1 starter set required by
-- STORAGE deliverable 1: glucose, heart_rate (both instantaneous and series),
-- temperature, medication_taken (= medication_dose), symptom, food (= meal +
-- food_item), plus a few neighbours that are referenced by the smoke test
-- corpus (blood_pressure, mood for sensitivity coverage).
--
-- Stable IDs are NOT pinned here yet (autoincrement); the runtime registry
-- looks up by (namespace, name) anyway. Future migration introduces frozen
-- IDs once exports/imports cross deployments.
--
-- Idempotent — uses INSERT OR IGNORE everywhere.

-- ----------- std.blood_glucose -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'blood_glucose', 'Blood glucose measurement', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'value', 'value', 'real', 'mmol/L', 'general' FROM event_types WHERE namespace='std' AND name='blood_glucose';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, enum_values, sensitivity_class)
SELECT id, NULL, 'measurement_method', 'measurement_method', 'enum', '["cgm","fingerstick","lab","unknown"]', 'general' FROM event_types WHERE namespace='std' AND name='blood_glucose';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, enum_values, sensitivity_class)
SELECT id, NULL, 'meal_relation', 'meal_relation', 'enum', '["fasting","pre_meal","post_meal","bedtime","random"]', 'general' FROM event_types WHERE namespace='std' AND name='blood_glucose';

-- Convenience alias 'std.glucose' → 'std.blood_glucose' for the smoke test
-- (deliverable 5 mentions `std.glucose`).
INSERT OR IGNORE INTO type_aliases (old_namespace, old_name, new_event_type_id)
SELECT 'std', 'glucose', id FROM event_types WHERE namespace='std' AND name='blood_glucose';

-- A second 'value_mg_per_dl' channel is offered for the smoke test convenience.
-- Storage stores in canonical mmol/L; this channel exists as an alias mapping
-- on writes (handled in events.rs unit conversion helper).
-- For now, keep it simple: smoke test writes `value` directly in mmol/L.

-- ----------- std.heart_rate_resting -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'heart_rate_resting', 'Resting heart rate measurement', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'value', 'value', 'real', 'bpm', 'general' FROM event_types WHERE namespace='std' AND name='heart_rate_resting';

-- ----------- std.heart_rate_series (sample-block) -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'heart_rate_series', 'Continuous heart rate samples (workout, ambulatory)', 'biometric');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'bpm', 'bpm', 'real', 'bpm', 'biometric' FROM event_types WHERE namespace='std' AND name='heart_rate_series';

-- Top-level "std.heart_rate" alias used colloquially.
INSERT OR IGNORE INTO type_aliases (old_namespace, old_name, new_event_type_id)
SELECT 'std', 'heart_rate', id FROM event_types WHERE namespace='std' AND name='heart_rate_resting';

-- ----------- std.body_temperature -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'body_temperature', 'Body temperature measurement', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'value', 'value', 'real', 'C', 'general' FROM event_types WHERE namespace='std' AND name='body_temperature';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, enum_values, sensitivity_class)
SELECT id, NULL, 'location', 'location', 'enum', '["oral","axillary","tympanic","temporal","rectal","forehead"]', 'general' FROM event_types WHERE namespace='std' AND name='body_temperature';

INSERT OR IGNORE INTO type_aliases (old_namespace, old_name, new_event_type_id)
SELECT 'std', 'temperature', id FROM event_types WHERE namespace='std' AND name='body_temperature';

-- ----------- std.blood_pressure -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'blood_pressure', 'Blood pressure measurement', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'systolic', 'systolic', 'real', 'mmHg', 'general' FROM event_types WHERE namespace='std' AND name='blood_pressure';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'diastolic', 'diastolic', 'real', 'mmHg', 'general' FROM event_types WHERE namespace='std' AND name='blood_pressure';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'pulse', 'pulse', 'real', 'bpm', 'general' FROM event_types WHERE namespace='std' AND name='blood_pressure';

-- ----------- std.medication_dose (= "std.medication_taken") -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'medication_dose', 'Recorded medication dose taken/skipped', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'name', 'name', 'text', NULL, 'general' FROM event_types WHERE namespace='std' AND name='medication_dose';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'dose', 'dose', 'real', NULL, 'general' FROM event_types WHERE namespace='std' AND name='medication_dose';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, enum_values, sensitivity_class)
SELECT id, NULL, 'dose_unit', 'dose_unit', 'enum', '["mg","mcg","g","ml","units","tablets","puffs","drops"]', 'general' FROM event_types WHERE namespace='std' AND name='medication_dose';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, enum_values, sensitivity_class)
SELECT id, NULL, 'status', 'status', 'enum', '["taken","skipped","late","refused"]', 'general' FROM event_types WHERE namespace='std' AND name='medication_dose';

INSERT OR IGNORE INTO type_aliases (old_namespace, old_name, new_event_type_id)
SELECT 'std', 'medication_taken', id FROM event_types WHERE namespace='std' AND name='medication_dose';

-- ----------- std.symptom -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'symptom', 'Reported symptom', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'name', 'name', 'text', NULL, 'general' FROM event_types WHERE namespace='std' AND name='symptom';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'severity', 'severity', 'int', NULL, 'general' FROM event_types WHERE namespace='std' AND name='symptom';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'location', 'location', 'text', NULL, 'general' FROM event_types WHERE namespace='std' AND name='symptom';

-- ----------- std.meal (food group) -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'meal', 'Logged meal with nutrition channels', 'lifestyle');

-- nutrition group (top-level)
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'nutrition', 'nutrition', 'group', NULL, 'lifestyle' FROM event_types WHERE namespace='std' AND name='meal';

-- nutrition.energy_kcal
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT et.id,
       (SELECT id FROM channels WHERE event_type_id=et.id AND path='nutrition'),
       'energy_kcal', 'nutrition.energy_kcal', 'real', 'kcal', 'lifestyle'
FROM event_types et WHERE et.namespace='std' AND et.name='meal';

-- nutrition.fat group
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT et.id,
       (SELECT id FROM channels WHERE event_type_id=et.id AND path='nutrition'),
       'fat', 'nutrition.fat', 'group', NULL, 'lifestyle'
FROM event_types et WHERE et.namespace='std' AND et.name='meal';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT et.id,
       (SELECT id FROM channels WHERE event_type_id=et.id AND path='nutrition.fat'),
       'total', 'nutrition.fat.total', 'real', 'g', 'lifestyle'
FROM event_types et WHERE et.namespace='std' AND et.name='meal';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT et.id,
       (SELECT id FROM channels WHERE event_type_id=et.id AND path='nutrition.fat'),
       'saturated', 'nutrition.fat.saturated', 'real', 'g', 'lifestyle'
FROM event_types et WHERE et.namespace='std' AND et.name='meal';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT et.id,
       (SELECT id FROM channels WHERE event_type_id=et.id AND path='nutrition.fat'),
       'trans', 'nutrition.fat.trans', 'real', 'g', 'lifestyle'
FROM event_types et WHERE et.namespace='std' AND et.name='meal';

-- nutrition.carbohydrates group
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT et.id,
       (SELECT id FROM channels WHERE event_type_id=et.id AND path='nutrition'),
       'carbohydrates', 'nutrition.carbohydrates', 'group', NULL, 'lifestyle'
FROM event_types et WHERE et.namespace='std' AND et.name='meal';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT et.id,
       (SELECT id FROM channels WHERE event_type_id=et.id AND path='nutrition.carbohydrates'),
       'total', 'nutrition.carbohydrates.total', 'real', 'g', 'lifestyle'
FROM event_types et WHERE et.namespace='std' AND et.name='meal';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT et.id,
       (SELECT id FROM channels WHERE event_type_id=et.id AND path='nutrition.carbohydrates'),
       'fiber', 'nutrition.carbohydrates.fiber', 'real', 'g', 'lifestyle'
FROM event_types et WHERE et.namespace='std' AND et.name='meal';

-- nutrition.protein
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT et.id,
       (SELECT id FROM channels WHERE event_type_id=et.id AND path='nutrition'),
       'protein', 'nutrition.protein', 'real', 'g', 'lifestyle'
FROM event_types et WHERE et.namespace='std' AND et.name='meal';

INSERT OR IGNORE INTO type_aliases (old_namespace, old_name, new_event_type_id)
SELECT 'std', 'food', id FROM event_types WHERE namespace='std' AND name='meal';

-- ----------- std.mood (mental_health) -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'mood', 'Subjective mood report', 'mental_health');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, enum_values, sensitivity_class)
SELECT id, NULL, 'mood', 'mood', 'enum', '["very_low","low","neutral","good","very_good"]', 'mental_health' FROM event_types WHERE namespace='std' AND name='mood';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, enum_values, sensitivity_class)
SELECT id, NULL, 'energy', 'energy', 'enum', '["exhausted","low","neutral","good","high"]', 'mental_health' FROM event_types WHERE namespace='std' AND name='mood';
