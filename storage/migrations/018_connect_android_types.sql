-- Connect Android event types.
--
-- Registers the namespaces the Connect Android app writes into directly:
-- `measurement.*` (BP / glucose / weight / temperature / heart rate / SpO2),
-- `medication.*`, `food.*`, `symptom.*`, and `activity.*` (steps / sleep).
--
-- Originally these were rejected by the storage core with `UnknownType`
-- because the seed in `002_std_registry.sql` only registered `std.*` rows
-- (blood_glucose / blood_pressure / heart_rate_resting / body_temperature /
-- medication_dose / symptom / meal / mood). Rather than aliasing every
-- channel back to a canonical std.* shape (which differs in places —
-- e.g. `std.blood_pressure.systolic` vs Connect's `systolic_mmhg`,
-- `std.symptom.severity:int` vs Connect's `severity:real`), we register
-- the Connect shapes as first-class event types in their own namespaces.
-- The two registries coexist; spec/data-model.md documents the canonical
-- mappings for cross-source aggregation later.
--
-- Idempotent — every INSERT is OR IGNORE.

-- ===========================================================================
-- measurement.*
-- ===========================================================================

-- ----------- measurement.blood_pressure -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('measurement', 'blood_pressure', 'Blood pressure (Connect Android)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'systolic_mmhg', 'systolic_mmhg', 'real', 'mmHg', 'general'
FROM event_types WHERE namespace='measurement' AND name='blood_pressure';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'diastolic_mmhg', 'diastolic_mmhg', 'real', 'mmHg', 'general'
FROM event_types WHERE namespace='measurement' AND name='blood_pressure';

-- ----------- measurement.glucose -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('measurement', 'glucose', 'Blood glucose (Connect Android)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'value', 'value', 'real', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='glucose';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'unit', 'unit', 'text', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='glucose';

-- ----------- measurement.weight -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('measurement', 'weight', 'Body weight (Connect Android + Health Connect)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'value', 'value', 'real', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='weight';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'unit', 'unit', 'text', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='weight';

-- Health Connect maps WeightRecord → measurement.weight with `kg` channel.
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'kg', 'kg', 'real', 'kg', 'general'
FROM event_types WHERE namespace='measurement' AND name='weight';

-- ----------- measurement.temperature -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('measurement', 'temperature', 'Body temperature (Connect Android + Health Connect)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'value', 'value', 'real', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='temperature';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'unit', 'unit', 'text', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='temperature';

-- Health Connect maps BodyTemperatureRecord → measurement.temperature with `celsius`.
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'celsius', 'celsius', 'real', 'C', 'general'
FROM event_types WHERE namespace='measurement' AND name='temperature';

-- ----------- measurement.heart_rate (Health Connect, per-sample) -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('measurement', 'heart_rate', 'Heart rate sample (Health Connect)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'bpm', 'bpm', 'real', 'bpm', 'general'
FROM event_types WHERE namespace='measurement' AND name='heart_rate';

-- ----------- measurement.urine_strip (4-analyte panel) -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('measurement', 'urine_strip', 'Urine dipstick reading (Connect Android UrineStripScreen)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'glucose',     'glucose',     'text', NULL, 'general' FROM event_types WHERE namespace='measurement' AND name='urine_strip';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'ph',          'ph',          'text', NULL, 'general' FROM event_types WHERE namespace='measurement' AND name='urine_strip';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'protein',     'protein',     'text', NULL, 'general' FROM event_types WHERE namespace='measurement' AND name='urine_strip';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'leukocytes',  'leukocytes',  'text', NULL, 'general' FROM event_types WHERE namespace='measurement' AND name='urine_strip';

-- ----------- measurement.spo2 (Health Connect) -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('measurement', 'spo2', 'Oxygen saturation (Health Connect)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'percentage', 'percentage', 'real', '%', 'general'
FROM event_types WHERE namespace='measurement' AND name='spo2';

-- ----------- measurement.ecg_second -----------
--
-- One event per second of ECG strip imported from external sources (today:
-- Samsung Health Monitor CSV export). A 30-second strip becomes 30 events,
-- joined by `correlation_id` (one ULID-shaped string per strip).
--
-- Per-second granularity instead of one big blob so:
--   1. The waveform can be queried against minute-level events naturally.
--   2. Each event payload stays small (500 samples × ~7 chars = ~3.5 KB).
--   3. Aggregate views can decimate / re-window without re-importing.
--
-- Channels:
--   - `correlation_id` — joins seconds of the same strip
--   - `second_index`   — 0-based offset within the strip
--   - `samples_mv`     — comma-separated mV floats (`sampling_rate_hz` per second)
--   - `sampling_rate_hz` — usually 500 (replicated on every second for self-containment)
--   - `lead`           — typically "Lead I" for wrist-watch ECG
--   - `avg_heart_rate` — strip-level avg (replicated)
--   - `classification` — "Sinus rhythm" / "Atrial fibrillation" / "Inconclusive" / …
--   - `symptoms`       — user-entered free text from Samsung's prompt
--   - `device`         — source device label (e.g. "Galaxy Watch6 Classic")
--   - `software_version` — exporter version
--   - `source_kind`    — provenance hint ("samsung_health_monitor", "apple_health", …)
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('measurement', 'ecg_second', 'One second of ECG waveform (per-second event)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'correlation_id',   'correlation_id',   'text', NULL,  'general' FROM event_types WHERE namespace='measurement' AND name='ecg_second';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'second_index',     'second_index',     'int',  NULL,  'general' FROM event_types WHERE namespace='measurement' AND name='ecg_second';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'samples_mv',       'samples_mv',       'text', 'mV',  'general' FROM event_types WHERE namespace='measurement' AND name='ecg_second';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'sampling_rate_hz', 'sampling_rate_hz', 'real', 'Hz',  'general' FROM event_types WHERE namespace='measurement' AND name='ecg_second';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'lead',             'lead',             'text', NULL,  'general' FROM event_types WHERE namespace='measurement' AND name='ecg_second';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'avg_heart_rate',   'avg_heart_rate',   'real', 'bpm', 'general' FROM event_types WHERE namespace='measurement' AND name='ecg_second';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'classification',   'classification',   'text', NULL,  'general' FROM event_types WHERE namespace='measurement' AND name='ecg_second';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'symptoms',         'symptoms',         'text', NULL,  'general' FROM event_types WHERE namespace='measurement' AND name='ecg_second';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'device',           'device',           'text', NULL,  'general' FROM event_types WHERE namespace='measurement' AND name='ecg_second';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'software_version', 'software_version', 'text', NULL,  'general' FROM event_types WHERE namespace='measurement' AND name='ecg_second';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'source_kind',      'source_kind',      'text', NULL,  'general' FROM event_types WHERE namespace='measurement' AND name='ecg_second';

-- ----------- measurement.pain (Connect Android NRS 0-10) -----------
--
-- Connect Android's PainScoreScreen writes 11-point Numeric Rating Scale
-- entries here. Three channels: `location` (free-text body site),
-- `severity_nrs` (0-10 real), `severity_label` (bucket label —
-- "No pain" / "Mild" / "Annoying" / "Distracting" / "Disabling" /
-- "Unbearable" / "Worst possible").
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('measurement', 'pain', 'Pain score NRS 0-10 (Connect Android)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'location', 'location', 'text', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='pain';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'severity_nrs', 'severity_nrs', 'real', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='pain';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'severity_label', 'severity_label', 'text', NULL, 'general'
FROM event_types WHERE namespace='measurement' AND name='pain';

-- ===========================================================================
-- medication.*
-- ===========================================================================

-- ----------- medication.taken -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('medication', 'taken', 'Medication dose taken (Connect Android)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'name', 'med.name', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='taken';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'dose', 'med.dose', 'real', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='taken';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'unit', 'med.unit', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='taken';

-- ===========================================================================
-- food.*
-- ===========================================================================

-- ----------- food.eaten -----------
-- Flat shape (kcal/carbs/protein/fat/sugar as siblings) per the Connect
-- Android FoodDetail screen; `std.meal` carries the canonical hierarchical
-- nutrition group separately.
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('food', 'eaten', 'Food intake (Connect Android FoodDetail)', 'lifestyle');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'name', 'name', 'text', NULL, 'lifestyle'
FROM event_types WHERE namespace='food' AND name='eaten';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'grams', 'grams', 'real', 'g', 'lifestyle'
FROM event_types WHERE namespace='food' AND name='eaten';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'kcal', 'kcal', 'real', 'kcal', 'lifestyle'
FROM event_types WHERE namespace='food' AND name='eaten';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'carbs_g', 'carbs_g', 'real', 'g', 'lifestyle'
FROM event_types WHERE namespace='food' AND name='eaten';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'protein_g', 'protein_g', 'real', 'g', 'lifestyle'
FROM event_types WHERE namespace='food' AND name='eaten';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'fat_g', 'fat_g', 'real', 'g', 'lifestyle'
FROM event_types WHERE namespace='food' AND name='eaten';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'sugar_g', 'sugar_g', 'real', 'g', 'lifestyle'
FROM event_types WHERE namespace='food' AND name='eaten';

-- Extended micronutrients + OFF composition channels (opportunistic — only
-- emitted when OpenFoodFacts had a non-zero value or a non-empty tag list).
-- Keep this list in sync with `FoodDetailScreen.foodChannels`.
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'fiber_g',          'fiber_g',          'real', 'g',  'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'saturated_fat_g',  'saturated_fat_g',  'real', 'g',  'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'trans_fat_g',      'trans_fat_g',      'real', 'g',  'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'sodium_mg',        'sodium_mg',        'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'cholesterol_mg',   'cholesterol_mg',   'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'potassium_mg',     'potassium_mg',     'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'calcium_mg',       'calcium_mg',       'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'iron_mg',          'iron_mg',          'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'vitamin_c_mg',     'vitamin_c_mg',     'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'vitamin_d_mcg',    'vitamin_d_mcg',    'real', 'mcg','lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'caffeine_mg',      'caffeine_mg',      'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
-- Composition (CSV text on the wire — keeps the channel surface flat).
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'additives',            'additives',            'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'allergens',            'allergens',            'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'traces',               'traces',               'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'ingredients_analysis', 'ingredients_analysis', 'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'labels',               'labels',               'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'nova_group',           'nova_group',           'int',  NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'nutri_score',          'nutri_score',          'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'eco_score',            'eco_score',            'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'off_data',             'off_data',             'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='eaten';

-- ----------- food.consumption_started / food.consumption_finished -----------
-- Marks gradual food/drink consumption: started (open Red Bull, brewed coffee)
-- and later finished. Pair is joined client-side via the `correlation_id`
-- text channel (a ULID minted at start time).
--
-- Started carries the same nutritional channels as `food.eaten` so the
-- Recent / Today panels can compute completed-consumption totals from the
-- *finished* event alone (we treat finished events as a fully-consumed
-- food.eaten with the started event's macros).
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('food', 'consumption_started', 'Food consumption started (gradual / sipped)', 'lifestyle');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'name', 'name', 'text', NULL, 'lifestyle'
FROM event_types WHERE namespace='food' AND name='consumption_started';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'grams', 'grams', 'real', 'g', 'lifestyle'
FROM event_types WHERE namespace='food' AND name='consumption_started';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'kcal', 'kcal', 'real', 'kcal', 'lifestyle'
FROM event_types WHERE namespace='food' AND name='consumption_started';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'carbs_g', 'carbs_g', 'real', 'g', 'lifestyle'
FROM event_types WHERE namespace='food' AND name='consumption_started';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'protein_g', 'protein_g', 'real', 'g', 'lifestyle'
FROM event_types WHERE namespace='food' AND name='consumption_started';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'fat_g', 'fat_g', 'real', 'g', 'lifestyle'
FROM event_types WHERE namespace='food' AND name='consumption_started';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'sugar_g', 'sugar_g', 'real', 'g', 'lifestyle'
FROM event_types WHERE namespace='food' AND name='consumption_started';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'correlation_id', 'correlation_id', 'text', NULL, 'lifestyle'
FROM event_types WHERE namespace='food' AND name='consumption_started';

-- Same extended channel set as `food.eaten` (started events carry the full
-- nutrition + OFF composition snapshot so the finished event only has to
-- reference the correlation_id and the actual_grams).
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'fiber_g',          'fiber_g',          'real', 'g',  'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'saturated_fat_g',  'saturated_fat_g',  'real', 'g',  'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'trans_fat_g',      'trans_fat_g',      'real', 'g',  'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'sodium_mg',        'sodium_mg',        'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'cholesterol_mg',   'cholesterol_mg',   'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'potassium_mg',     'potassium_mg',     'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'calcium_mg',       'calcium_mg',       'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'iron_mg',          'iron_mg',          'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'vitamin_c_mg',     'vitamin_c_mg',     'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'vitamin_d_mcg',    'vitamin_d_mcg',    'real', 'mcg','lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'caffeine_mg',      'caffeine_mg',      'real', 'mg', 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'additives',            'additives',            'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'allergens',            'allergens',            'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'traces',               'traces',               'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'ingredients_analysis', 'ingredients_analysis', 'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'labels',               'labels',               'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'nova_group',           'nova_group',           'int',  NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'nutri_score',          'nutri_score',          'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'eco_score',            'eco_score',            'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'off_data',             'off_data',             'text', NULL, 'lifestyle' FROM event_types WHERE namespace='food' AND name='consumption_started';

INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('food', 'consumption_finished', 'Food consumption finished (matches started by correlation_id)', 'lifestyle');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'correlation_id', 'correlation_id', 'text', NULL, 'lifestyle'
FROM event_types WHERE namespace='food' AND name='consumption_finished';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'actual_grams', 'actual_grams', 'real', 'g', 'lifestyle'
FROM event_types WHERE namespace='food' AND name='consumption_finished';

-- ===========================================================================
-- symptom.*
-- ===========================================================================
--
-- Connect Android's symptom logger uses `symptom.<snake_name>` as the event
-- type itself (e.g. `symptom.headache`, `symptom.fatigue`) so the
-- per-symptom timelines stay queryable without a name-channel filter. We
-- pre-register the 15 default presets; users can also write `symptom.other`
-- for free-text input. The shared channel set across all variants:
-- `severity` (real, 0–10 NRS), `severity_label` (text), `notes` (text).

-- helper macro substitute via repeated blocks. Listed in DefaultSymptoms
-- order from `SymptomLogScreen.kt`.

INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class) VALUES
    ('symptom', 'headache',            'Headache',            'general'),
    ('symptom', 'migraine',            'Migraine',            'general'),
    ('symptom', 'fatigue',             'Fatigue',             'general'),
    ('symptom', 'nausea',              'Nausea',              'general'),
    ('symptom', 'dizziness',           'Dizziness',           'general'),
    ('symptom', 'cough',               'Cough',               'general'),
    ('symptom', 'sore_throat',         'Sore throat',         'general'),
    ('symptom', 'fever',               'Fever',               'general'),
    ('symptom', 'stomach_pain',        'Stomach pain',        'general'),
    ('symptom', 'joint_pain',          'Joint pain',          'general'),
    ('symptom', 'back_pain',           'Back pain',           'general'),
    ('symptom', 'shortness_of_breath', 'Shortness of breath', 'general'),
    ('symptom', 'anxiety',             'Anxiety',             'mental_health'),
    ('symptom', 'insomnia',            'Insomnia',            'general'),
    ('symptom', 'other',               'Other (free-text)',   'general');

-- Add the three shared channels to every symptom.* type.
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'severity', 'severity', 'real', NULL, default_sensitivity_class
FROM event_types WHERE namespace='symptom';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'severity_label', 'severity_label', 'text', NULL, default_sensitivity_class
FROM event_types WHERE namespace='symptom';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'notes', 'notes', 'text', NULL, default_sensitivity_class
FROM event_types WHERE namespace='symptom';

-- ===========================================================================
-- activity.* (Health Connect)
-- ===========================================================================

-- ----------- activity.steps -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('activity', 'steps', 'Step count (Health Connect)', 'lifestyle');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'count', 'count', 'int', NULL, 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='steps';

-- ----------- activity.sleep -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('activity', 'sleep', 'Sleep session (Health Connect)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'duration_minutes', 'duration_minutes', 'int', 'min', 'general'
FROM event_types WHERE namespace='activity' AND name='sleep';

-- ----------- Health Connect — extended measurement.* (per-record types) -----------
-- Channel paths mirror the writes in `HealthConnectSync.kt`. `resolve_event_type`
-- doesn't auto-register, so every type the sync writes must be listed here.
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class) VALUES
    ('measurement', 'basal_body_temperature', 'Basal body temperature (Health Connect)', 'general'),
    ('measurement', 'resting_heart_rate',     'Resting heart rate (Health Connect)',     'general'),
    ('measurement', 'hrv_rmssd',              'Heart-rate variability RMSSD (Health Connect)', 'general'),
    ('measurement', 'respiratory_rate',       'Respiratory rate (Health Connect)',       'general'),
    ('measurement', 'height',                 'Height (Health Connect)',                 'general'),
    ('measurement', 'body_fat',               'Body fat percentage (Health Connect)',    'general'),
    ('measurement', 'body_water_mass',        'Body water mass (Health Connect)',        'general'),
    ('measurement', 'bone_mass',              'Bone mass (Health Connect)',              'general'),
    ('measurement', 'lean_body_mass',         'Lean body mass (Health Connect)',         'general'),
    ('measurement', 'basal_metabolic_rate',   'Basal metabolic rate (Health Connect)',   'general'),
    ('measurement', 'vo2_max',                'VO2 max (Health Connect)',                'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'celsius', 'celsius', 'real', 'C', 'general'
FROM event_types WHERE namespace='measurement' AND name='basal_body_temperature';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'bpm', 'bpm', 'real', 'bpm', 'general'
FROM event_types WHERE namespace='measurement' AND name='resting_heart_rate';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'rmssd_ms', 'rmssd_ms', 'real', 'ms', 'general'
FROM event_types WHERE namespace='measurement' AND name='hrv_rmssd';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'rate_per_min', 'rate_per_min', 'real', '1/min', 'general'
FROM event_types WHERE namespace='measurement' AND name='respiratory_rate';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'meters', 'meters', 'real', 'm', 'general'
FROM event_types WHERE namespace='measurement' AND name='height';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'percentage', 'percentage', 'real', '%', 'general'
FROM event_types WHERE namespace='measurement' AND name='body_fat';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'kg', 'kg', 'real', 'kg', 'general'
FROM event_types WHERE namespace='measurement' AND name='body_water_mass';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'kg', 'kg', 'real', 'kg', 'general'
FROM event_types WHERE namespace='measurement' AND name='bone_mass';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'kg', 'kg', 'real', 'kg', 'general'
FROM event_types WHERE namespace='measurement' AND name='lean_body_mass';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'kcal_per_day', 'kcal_per_day', 'real', 'kcal/d', 'general'
FROM event_types WHERE namespace='measurement' AND name='basal_metabolic_rate';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'ml_per_kg_per_min', 'ml_per_kg_per_min', 'real', 'mL/kg/min', 'general'
FROM event_types WHERE namespace='measurement' AND name='vo2_max';

-- ----------- Health Connect — extended activity.* -----------
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class) VALUES
    ('activity', 'distance',               'Distance moved (Health Connect)',          'lifestyle'),
    ('activity', 'elevation_gained',       'Elevation gained (Health Connect)',        'lifestyle'),
    ('activity', 'floors_climbed',         'Floors climbed (Health Connect)',          'lifestyle'),
    ('activity', 'active_calories_burned', 'Active calories burned (Health Connect)',  'lifestyle'),
    ('activity', 'total_calories_burned',  'Total calories burned (Health Connect)',   'lifestyle'),
    ('activity', 'exercise_session',       'Exercise session (Health Connect)',        'lifestyle'),
    ('activity', 'power',                  'Power sample (Health Connect)',            'lifestyle'),
    ('activity', 'speed',                  'Speed sample (Health Connect)',            'lifestyle'),
    ('activity', 'steps_cadence',          'Step cadence sample (Health Connect)',     'lifestyle'),
    ('activity', 'cycling_cadence',        'Cycling cadence sample (Health Connect)',  'lifestyle'),
    ('activity', 'wheelchair_pushes',      'Wheelchair pushes (Health Connect)',       'lifestyle'),
    ('activity', 'hydration',              'Fluid intake (Health Connect)',            'lifestyle');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'meters', 'meters', 'real', 'm', 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='distance';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'meters', 'meters', 'real', 'm', 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='elevation_gained';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'count', 'count', 'real', NULL, 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='floors_climbed';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'kcal', 'kcal', 'real', 'kcal', 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='active_calories_burned';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'kcal', 'kcal', 'real', 'kcal', 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='total_calories_burned';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'exercise_type', 'exercise_type', 'int', NULL, 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='exercise_session';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'title', 'title', 'text', NULL, 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='exercise_session';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'watts', 'watts', 'real', 'W', 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='power';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'm_per_s', 'm_per_s', 'real', 'm/s', 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='speed';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'steps_per_min', 'steps_per_min', 'real', '1/min', 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='steps_cadence';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'rpm', 'rpm', 'real', '1/min', 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='cycling_cadence';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'count', 'count', 'int', NULL, 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='wheelchair_pushes';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'liters', 'liters', 'real', 'L', 'lifestyle'
FROM event_types WHERE namespace='activity' AND name='hydration';

-- ===========================================================================
-- intake.* — per-nutrient child events emitted by FoodDetailScreen
-- (`emitIntakeChildren`). Each carries `value:real`, `unit:text`, and
-- `correlation_id:text` so totals are a flat type-filtered sum and drill-down
-- back to the parent food.eaten is one correlation_id lookup.
-- ===========================================================================

INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class) VALUES
    ('intake', 'kcal',             'Calories ingested',           'lifestyle'),
    ('intake', 'carbs_g',          'Carbohydrates ingested (g)',  'lifestyle'),
    ('intake', 'protein_g',        'Protein ingested (g)',        'lifestyle'),
    ('intake', 'fat_g',            'Fat ingested (g)',            'lifestyle'),
    ('intake', 'sugar_g',          'Sugar ingested (g)',          'lifestyle'),
    ('intake', 'fiber_g',          'Fiber ingested (g)',          'lifestyle'),
    ('intake', 'saturated_fat_g',  'Saturated fat ingested (g)',  'lifestyle'),
    ('intake', 'trans_fat_g',      'Trans fat ingested (g)',      'lifestyle'),
    ('intake', 'sodium_mg',        'Sodium ingested (mg)',        'lifestyle'),
    ('intake', 'cholesterol_mg',   'Cholesterol ingested (mg)',   'lifestyle'),
    ('intake', 'potassium_mg',     'Potassium ingested (mg)',     'lifestyle'),
    ('intake', 'calcium_mg',       'Calcium ingested (mg)',       'lifestyle'),
    ('intake', 'iron_mg',          'Iron ingested (mg)',          'lifestyle'),
    ('intake', 'vitamin_c_mg',     'Vitamin C ingested (mg)',     'lifestyle'),
    ('intake', 'vitamin_d_mcg',    'Vitamin D ingested (mcg)',    'lifestyle'),
    ('intake', 'caffeine_mg',      'Caffeine ingested (mg)',      'lifestyle');

-- Shared channel set on every intake.* type.
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'value', 'value', 'real', NULL, default_sensitivity_class
FROM event_types WHERE namespace='intake';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'unit', 'unit', 'text', NULL, default_sensitivity_class
FROM event_types WHERE namespace='intake';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'correlation_id', 'correlation_id', 'text', NULL, default_sensitivity_class
FROM event_types WHERE namespace='intake';

-- ===========================================================================
-- std.clinical_note
--
-- Referenced by the GrantTemplates `PRIMARY_DOCTOR` and `SPECIALIST_VISIT`
-- templates as a read/write scope; grant creation rejected the templates
-- with `UnknownType("std.clinical_note")` before this row landed. Channels
-- are minimal — title/body/author — so a doctor's free-text note round-trips
-- through `put_event` / `query_events`.
-- ===========================================================================

INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('std', 'clinical_note', 'Clinical note / doctor free-text observation', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'title', 'title', 'text', NULL, 'general'
FROM event_types WHERE namespace='std' AND name='clinical_note';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'body', 'body', 'text', NULL, 'general'
FROM event_types WHERE namespace='std' AND name='clinical_note';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'author', 'author', 'text', NULL, 'general'
FROM event_types WHERE namespace='std' AND name='clinical_note';

-- ===========================================================================
-- audit.* — supersede pointer for edit-then-save.
-- ===========================================================================
--
-- The Connect Android EditEventScreen never mutates an existing row (uniffi
-- exposes no `update_event` / `delete_event` RPC); instead it appends a
-- corrected event with `source = "manual:android_app"` and a pointer
-- event of type `audit.event_superseded` carrying both ULIDs. A future
-- operator screen can chain originals → corrections by querying this
-- type, and the original row stays untouched for traceability.

INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('audit', 'event_superseded', 'Pointer from corrected event to its replacement', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'original_ulid', 'original_ulid', 'text', NULL, 'general'
FROM event_types WHERE namespace='audit' AND name='event_superseded';

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'new_ulid', 'new_ulid', 'text', NULL, 'general'
FROM event_types WHERE namespace='audit' AND name='event_superseded';

-- ===========================================================================
-- emergency.* — operator-side audit trail for the break-glass feature.
-- ===========================================================================

-- ----------- emergency.test_run -----------
-- Logged by the "Run test alert" button on EmergencySettingsScreen so the
-- user can rehearse what responders would see without firing a real BLE
-- emergency. Single `kind` channel (always "test" today; reserved for
-- "drill", "dry_run" in v0.x).
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('emergency', 'test_run', 'Emergency break-glass test alert (no responder involved)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'kind', 'kind', 'text', NULL, 'general'
FROM event_types WHERE namespace='emergency' AND name='test_run';
