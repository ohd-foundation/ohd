-- Clinical + persistent-fact event types. (Migration 020.)
--
-- Registers the namespaces for the persistent-facts / medication-regimen /
-- clinical-case subsystem (see plan deep-dancing-teacup.md):
--
--   profile.*     — persistent patient facts (blood type, allergies,
--                   conditions, emergency contacts, advance directives).
--                   Modelled as typed per-fact events: each edit writes a
--                   new event with a stable `fact_id` + `status`; current
--                   state is "latest event per fact_id, drop removed".
--   medication.regimen_started / _discontinued — a prescribed/taken
--                   course of a drug. Doses (medication.taken) link to a
--                   regimen by `regimen_id`. Active = started-without-
--                   discontinued.
--   clinical.*    — episode records (doctor visit, prescription, lab
--                   result). Each carries a `case_id` channel tying it to a
--                   case (membership is enforced by a case-filter with a
--                   channel-predicate {case_id eq <ulid>}), plus
--                   `entered_by` provenance and a reserved `source_document`
--                   attachment-ULID channel (doc-scan deferred).
--
-- Without registration these would land under `custom.<dotted>` on first
-- write; pre-registering keeps the clean namespace + lets the read tools
-- query the types directly.
--
-- Sensitivity: all `general`. The recognized special classes
-- (mental_health / substance_use / sexual_health / reproductive) are
-- deny-categories applied per-grant — NOT something to bake into a
-- profile/clinical type, which would hide it from clinicians by default
-- and defeats the whole point (a clinician must see allergies + meds).
-- A specific sensitive condition gets its sensitivity at the per-grant
-- rule level, not here.
--
-- Idempotent — every INSERT is OR IGNORE.

-- ===========================================================================
-- profile.blood_type   (singleton — latest event wins, no fact_id)
-- ===========================================================================
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('profile', 'blood_type', 'ABO/Rh blood type', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'group', 'group', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='blood_type';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'rh', 'rh', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='blood_type';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'detail', 'detail', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='blood_type';

-- ===========================================================================
-- profile.allergy   (fact_id + status: active|removed)
-- ===========================================================================
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('profile', 'allergy', 'A recorded allergy (latest per fact_id wins)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'fact_id', 'fact_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='allergy';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'allergen', 'allergen', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='allergy';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'severity', 'severity', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='allergy';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'reaction', 'reaction', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='allergy';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'status', 'status', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='allergy';

-- ===========================================================================
-- profile.condition   (fact_id + status: active|resolved)
-- ===========================================================================
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('profile', 'condition', 'A diagnosed condition (latest per fact_id wins)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'fact_id', 'fact_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='condition';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'name', 'name', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='condition';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'icd10', 'icd10', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='condition';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'status', 'status', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='condition';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'onset_ms', 'onset_ms', 'int', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='condition';

-- ===========================================================================
-- profile.emergency_contact   (fact_id + status: active|removed)
-- ===========================================================================
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('profile', 'emergency_contact', 'An emergency contact (latest per fact_id wins)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'fact_id', 'fact_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='emergency_contact';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'name', 'name', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='emergency_contact';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'relation', 'relation', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='emergency_contact';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'phone', 'phone', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='emergency_contact';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'status', 'status', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='emergency_contact';

-- ===========================================================================
-- profile.advance_directive   (fact_id + status: active|removed)
-- ===========================================================================
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('profile', 'advance_directive', 'An advance directive (latest per fact_id wins)', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'fact_id', 'fact_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='advance_directive';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'kind', 'kind', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='advance_directive';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'detail', 'detail', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='advance_directive';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'status', 'status', 'text', NULL, 'general'
FROM event_types WHERE namespace='profile' AND name='advance_directive';

-- ===========================================================================
-- medication.regimen_started   (regimen_id minted at start; doses link to it)
-- ===========================================================================
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('medication', 'regimen_started', 'Start of a medication course', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'regimen_id', 'regimen_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='regimen_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'name', 'name', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='regimen_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'dose_value', 'dose_value', 'real', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='regimen_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'dose_unit', 'dose_unit', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='regimen_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'frequency', 'frequency', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='regimen_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'rx_concept_id', 'rx_concept_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='regimen_started';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'case_id', 'case_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='regimen_started';

-- ===========================================================================
-- medication.regimen_discontinued
-- ===========================================================================
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('medication', 'regimen_discontinued', 'End of a medication course', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'regimen_id', 'regimen_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='regimen_discontinued';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'reason', 'reason', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='regimen_discontinued';

-- ===========================================================================
-- medication.taken — extend with regimen link + actual-dose channels.
-- (The type itself was registered in 018; these channels auto-register on
-- first write, but we declare them here for the canonical dose-flexibility
-- shape: record ACTUAL dose, never the prescribed one. `skipped` is a
-- first-class, no-shame signal.)
-- ===========================================================================
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'regimen_id', 'regimen_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='taken';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'dose_value', 'dose_value', 'real', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='taken';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'dose_unit', 'dose_unit', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='taken';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'dose_note', 'dose_note', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='taken';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'skipped', 'skipped', 'bool', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='taken';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'adherence_reason', 'adherence_reason', 'text', NULL, 'general'
FROM event_types WHERE namespace='medication' AND name='taken';

-- ===========================================================================
-- clinical.visit / clinical.prescription / clinical.lab_result
-- All carry: case_id (membership), entered_by (provenance), source_document
-- (reserved attachment ULID — doc-scan deferred).
-- ===========================================================================
INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('clinical', 'visit', 'A doctor / clinic visit', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'case_id', 'case_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='visit';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'practitioner_name', 'practitioner_name', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='visit';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'specialty', 'specialty', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='visit';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'facility', 'facility', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='visit';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'reason', 'reason', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='visit';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'entered_by', 'entered_by', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='visit';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'source_document', 'source_document', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='visit';

INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('clinical', 'prescription', 'A prescription issued at a visit', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'case_id', 'case_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='prescription';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'medication_name', 'medication_name', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='prescription';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'dose_value', 'dose_value', 'real', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='prescription';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'dose_unit', 'dose_unit', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='prescription';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'frequency', 'frequency', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='prescription';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'duration_days', 'duration_days', 'int', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='prescription';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'regimen_id', 'regimen_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='prescription';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'entered_by', 'entered_by', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='prescription';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'source_document', 'source_document', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='prescription';

INSERT OR IGNORE INTO event_types (namespace, name, description, default_sensitivity_class)
VALUES ('clinical', 'lab_result', 'A lab test result', 'general');

INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'case_id', 'case_id', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='lab_result';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'test_name', 'test_name', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='lab_result';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'value', 'value', 'real', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='lab_result';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'value_text', 'value_text', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='lab_result';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'unit', 'unit', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='lab_result';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'reference_range', 'reference_range', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='lab_result';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'entered_by', 'entered_by', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='lab_result';
INSERT OR IGNORE INTO channels (event_type_id, parent_id, name, path, value_type, unit, sensitivity_class)
SELECT id, NULL, 'source_document', 'source_document', 'text', NULL, 'general'
FROM event_types WHERE namespace='clinical' AND name='lab_result';
