# Design: Event Vocabulary

> The catalog of **standard event types and channels** that ship with OHD Storage. The "what events exist" doc.
>
> Pairs with [`storage-format.md`](storage-format.md) (the on-disk schema and bit-level format) and the OHDC `.proto` (the wire format — see [`../components/connect.md`](../components/connect.md) "Wire format"). This doc is the conceptual vocabulary; those docs are the mechanics.

> **History:** earlier drafts of this file described a JSON event with `value`/`unit`/`data` JSONB columns on a Postgres backend. That was the v0 prototype model and is **superseded**. The current model is the typed channel-tree EAV described in [`storage-format.md`](storage-format.md). This file now contains only the vocabulary.

## How events are shaped (one-paragraph recap)

An OHD event is one occurrence at a `(timestamp_ms, optional duration_ms)` of a specific **event type** drawn from this registry. Each type has a tree of **channels** — typed, named, optionally-grouped scalar measurements. An event records zero or more of its type's leaf channels. Dense numeric streams (HR samples in a workout) are stored as **sample blocks** within an event. Large binary payloads (ECG raw, image, PDF) are **attachments** referenced by SHA-256.

Full mechanics, indexes, and the SQL schema live in [`storage-format.md`](storage-format.md).

## Naming conventions

- **Standard namespace** — `std`. Ships with the OHD format spec; identical across implementations. Stable IDs in the embedded registry catalog.
- **Custom namespace** — `com.<owner>.<name>`. Lives in the user's file alongside standard types; round-trips through export/import. Examples: `com.openhealth.skin_lesion`, `com.acme.implant_telemetry`. Owner is a domain or organization slug; name is the type's local identifier.
- **Channel paths** — dot-separated tree, e.g. `meal.nutrition.fat.saturated`. Group nodes (`is_group=1`) carry no value; only structure. Leaves carry typed values (`real` / `int` / `bool` / `text` / `enum`).
- **Units** — each channel declares one **canonical unit** (the unit values are stored in). Submissions in other units are rejected at write time with `INVALID_UNIT`; consumers convert to canonical before submitting. Avoids mmol-vs-mg-dL ambiguity at the storage layer forever.
- **Enums** — `enum_values` arrays are append-only; each entry's index is its on-disk ordinal forever. Renaming an enum value is allowed (display label only); reordering or removing is not.

## Sensitivity classes

Every event type and channel has a `sensitivity_class` (see [`storage-format.md`](storage-format.md) "Privacy is structural, not annotative"). Grants reference these classes when defining read scope. Standard catalog:

| Class | Examples |
|---|---|
| `general` | Most measurements, food, exercise, lab results |
| `biometric` | High-resolution biometric data (ECG raw, HR samples, glucose CGM streams) |
| `mental_health` | Mood, anxiety scores, mental-state observations, psychotherapy notes |
| `sexual_health` | Sexual activity, STI tests, contraception |
| `substance_use` | Alcohol, nicotine, recreational substances |
| `reproductive` | Menstrual cycle, pregnancy, fertility tracking |
| `lifestyle` | Food, hydration, exercise (separable from clinical-grade biometrics) |

A given event's effective sensitivity class comes from its event type's `default_sensitivity_class` plus the channel-level `sensitivity_class` of any present channels — `deny wins` on conflict per the resolution algorithm in [`storage-format.md`](storage-format.md).

---

## Standard catalog

The catalog below is normative for v1. New entries are additive; the registry version bumps when new entries are added (see [`storage-format.md`](storage-format.md) "Channel registry").

### Biometric measurements — instantaneous

#### `std.blood_glucose`
- `value` — real, **mmol/L**
- `measurement_method` — enum: `cgm`, `fingerstick`, `lab`, `unknown`
- `meal_relation` — enum: `fasting`, `pre_meal`, `post_meal`, `bedtime`, `random` (optional)

#### `std.blood_pressure`
- `systolic` — real, mmHg
- `diastolic` — real, mmHg
- `pulse` — real, bpm (optional; many cuffs report it)
- `position` — enum: `sitting`, `standing`, `supine` (optional)
- `arm` — enum: `left`, `right` (optional)

#### `std.body_temperature`
- `value` — real, **°C**
- `location` — enum: `oral`, `axillary`, `tympanic`, `temporal`, `rectal`, `forehead` (optional)

#### `std.oxygen_saturation`
- `value` — real, % (0–100)
- `pulse` — real, bpm (optional; oximeters report co-measured HR)

#### `std.respiratory_rate`
- `value` — real, breaths/minute

#### `std.heart_rate_resting`
- `value` — real, bpm

#### `std.heart_rate_variability`
- `rmssd` — real, ms (canonical HRV metric)
- `sdnn` — real, ms (optional)

#### `std.weight`
- `value` — real, **kg**

#### `std.height`
- `value` — real, **cm**

#### `std.body_composition`
- `fat_percent` — real, %
- `muscle_mass` — real, kg
- `bone_mass` — real, kg
- `water_percent` — real, %

#### `std.peak_flow`
- `value` — real, L/min

#### `std.blood_ketones`
- `value` — real, mmol/L

#### `std.urine_strip`
- `glucose` — enum: `negative`, `+`, `++`, `+++`, `++++`
- `protein` — enum: `negative`, `trace`, `+`, `++`, `+++`
- `ketones` — enum: `negative`, `+`, `++`, `+++`, `++++`
- `blood` — enum: `negative`, `trace`, `+`, `++`, `+++`
- `leukocytes` — enum: `negative`, `trace`, `+`, `++`, `+++`
- `nitrites` — enum: `negative`, `positive`
- `ph` — real (typically 4.5–8.0)
- `specific_gravity` — real (typically 1.005–1.030)
- `urobilinogen` — enum: `normal`, `+`, `++`, `+++`, `++++`
- `bilirubin` — enum: `negative`, `+`, `++`, `+++`

### Biometric measurements — continuous (sample blocks)

These types use the dense **sample blocks** mechanism from [`storage-format.md`](storage-format.md). Each event covers a time window (typically 15 min by default, configurable per channel); the block stores the `(t_offset_ms, value)` pairs compressed.

| Type | Sample channel | Unit | Sensitivity |
|---|---|---|---|
| `std.heart_rate_series` | `bpm` | bpm | `biometric` |
| `std.glucose_series` | `value` | mmol/L | `biometric` |
| `std.spo2_series` | `value` | % | `biometric` |
| `std.respiratory_rate_series` | `value` | breaths/min | `biometric` |
| `std.ecg_recording` | `µv` | µV | `biometric` (raw waveform usually as attachment when long) |

### Events with duration

#### `std.sleep`
- `quality` — enum: `very_poor`, `poor`, `fair`, `good`, `excellent` (subjective; optional)
- `stages` — sample-blocks-like channel encoding stage transitions (`awake`, `light`, `deep`, `rem`); see storage-format.md
- `interruptions` — int, count of mid-sleep wakings (optional)

#### `std.exercise`
- `activity` — enum: `running`, `cycling`, `swimming`, `walking`, `strength`, `yoga`, `hiit`, `hiking`, `other`
- `distance` — real, m (optional, activity-dependent)
- `calories` — real, kcal (optional)
- `active_minutes` — int (optional)
- `intensity` — enum: `low`, `moderate`, `high`, `vigorous` (optional)

#### `std.meal`
Channel tree — see [`storage-format.md`](storage-format.md) "Channels are a tree" for the full nested structure. Summary:

- `nutrition.energy_kcal` — real, kcal
- `nutrition.fat.total` — real, g
- `nutrition.fat.saturated` — real, g
- `nutrition.fat.unsaturated.mono` / `.poly` — real, g
- `nutrition.fat.trans` — real, g
- `nutrition.carbohydrates.total` — real, g
- `nutrition.carbohydrates.sugars.total` / `.added` — real, g
- `nutrition.carbohydrates.fiber` — real, g
- `nutrition.protein` — real, g
- `nutrition.salt` — real, g
- `notes` — text (optional)

Sensitivity: `lifestyle` by default.

#### `std.food_item`
A child of `meal` — for per-item logging when a meal has multiple components. Linked to the parent meal via `metadata.parent_event_ulid`. Same nutrition-tree channels.

#### `std.hospital_stay`
- `admission_reason` — text
- `discharge_summary` — text or attachment
- `facility` — text (operator-side identifier)

### Medications

#### `std.medication_prescribed`
- `name` — text (free-form for v1; controlled-vocabulary linkage TBD)
- `dose` — real
- `dose_unit` — enum: `mg`, `mcg`, `g`, `ml`, `units`, `tablets`, `puffs`, `drops`
- `schedule` — text (natural language for v1, e.g. "twice daily with meals"; structured schedule TBD)
- `prescribed_by` — text
- `reason` — text (optional)
- `rxnorm_id` — text (optional; for normalized lookups)

#### `std.medication_dose`
- `name` — text
- `dose` — real
- `dose_unit` — enum (same as `medication_prescribed`)
- `status` — enum: `taken`, `skipped`, `late`, `refused`
- `notes` — text (optional)
- `reference_prescription_ulid` — text (optional; links to a `medication_prescribed` event)

### Symptoms

#### `std.symptom`
- `name` — text (free-form for v1; SNOMED CT mapping deferred)
- `severity` — int, 1–10 scale (optional)
- `location` — text (e.g. "frontal", "left lower abdomen")
- `notes` — text

### Mental health  (sensitivity_class=`mental_health`)

#### `std.mood`
- `mood` — enum: `very_low`, `low`, `neutral`, `good`, `very_good`
- `energy` — enum: `exhausted`, `low`, `neutral`, `good`, `high` (optional)
- `notes` — text (optional)

#### `std.mental_state`
- `anxiety_level` — int, 0–10
- `depression_indicator` — int, 0–10
- `irritability` — int, 0–10 (optional)
- `notes` — text (optional)

#### `std.therapy_session`
- `provider` — text
- `notes` — text (often kept private; sensitivity ensures grants must explicitly include `mental_health`)

### Reproductive  (sensitivity_class=`reproductive`)

#### `std.menstrual_flow`
- `flow` — enum: `none`, `spotting`, `light`, `normal`, `heavy`

#### `std.cycle_observation`
- `cervical_mucus` — enum: `dry`, `sticky`, `creamy`, `watery`, `eggwhite` (optional)
- `basal_temp` — real, °C (optional)
- `ovulation_test` — enum: `positive`, `negative`, `peak` (optional)

#### `std.intermenstrual_bleeding`
- `severity` — enum: `spotting`, `light`, `moderate`, `heavy`

#### `std.pregnancy`
- `status` — enum: `confirmed`, `suspected`, `terminated`, `delivered`
- `notes` — text

### Sexual health  (sensitivity_class=`sexual_health`)

#### `std.sexual_activity`
- `protected` — bool (optional)
- `notes` — text (optional)

### Substance use  (sensitivity_class=`substance_use`)

#### `std.alcohol`
- `beverage` — text (e.g. "beer", "wine", "spirits")
- `volume_ml` — real
- `abv_percent` — real (optional, for accurate ethanol calc)
- `grams_ethanol` — real (computed if `volume_ml` + `abv_percent` provided)

#### `std.caffeine`
- `beverage` — text (e.g. "coffee", "tea", "energy_drink")
- `mg` — real

#### `std.nicotine`
- `product` — enum: `cigarette`, `cigar`, `pipe`, `vape`, `pouch`, `nrt`
- `count` — int (units-of-product; cigarettes, pouches, etc.)

#### `std.substance_use`
- `substance` — text
- `dose` — real
- `dose_unit` — enum: `mg`, `g`, `ml`, `units`, `pieces`
- `route` — enum: `oral`, `inhaled`, `intranasal`, `iv`, `im`, `transdermal`, `other` (optional)
- `notes` — text (optional)

### Lifestyle  (sensitivity_class=`lifestyle`)

#### `std.hydration`
- `ml` — real
- `beverage` — text (optional)

### Medical records (sensitivity_class=`general` unless noted)

#### `std.diagnosis`
- `condition_name` — text
- `icd10_code` — text (optional)
- `diagnosed_by` — text
- `notes` — text (optional)
- `status` — enum: `active`, `resolved`, `chronic`, `in_remission` (optional)

#### `std.lab_result`
- `test_name` — text
- `loinc_code` — text (optional)
- `value` — real
- `value_unit` — text (free-form; lab conventions vary widely)
- `reference_range` — text
- `notes` — text (optional)

#### `std.imaging`
- `modality` — enum: `MRI`, `CT`, `X-ray`, `US`, `PET`, `mammography`, `dexa`, `endoscopy`, `other`
- `body_region` — text
- `report` — text (radiologist's read)
- `image_attachment_ulid` — text (optional; references an attachment with the image/DICOM)

#### `std.procedure`
- `procedure_name` — text
- `cpt_code` — text (optional)
- `performed_by` — text
- `notes` — text (optional)

#### `std.vaccination`
- `vaccine_name` — text
- `dose_number` — int
- `lot_number` — text (optional)
- `given_by` — text
- `notes` — text (optional)

#### `std.allergy`
- `allergen` — text
- `severity` — enum: `mild`, `moderate`, `severe`, `anaphylactic`
- `reaction` — text

#### `std.clinical_note`
- `provider` — text
- `text` — text

#### `std.referral`
- `to_provider` — text
- `reason` — text
- `notes` — text (optional)

### Case lifecycle markers

These are events that record case state transitions for **timeline display**. Cases themselves are a separate primitive (see [`storage-format.md`](storage-format.md) "Cases"); these events are *not* how a case knows its own membership (that's `case_filters`). They simply make case lifecycle visible in the patient's chronological event view.

Each lifecycle marker references the case by ULID via the `case_ref_ulid` channel (text, the Crockford-base32 form of the case's ULID). Storage doesn't enforce referential integrity here — these events are informational; the canonical case state lives in the `cases` table.

#### `std.case_started`
- `case_ref_ulid` — text (the case's ULID)
- `kind` — enum: `clinical_visit`, `hospital_stay`, `episode_of_care`, `cycle`, `study`, `emergency`, `user_custom`
- `label` — text

#### `std.case_closed`
- `case_ref_ulid` — text
- `outcome` — text (optional)

#### `std.case_handoff`
- `case_ref_ulid` — text (the closing case)
- `successor_case_ref_ulid` — text (the new case)
- `handed_to_label` — text

### Free / custom

#### `std.note`
A general-purpose freeform timeline note. Use sparingly — prefer specific types so structured queries work. For genuinely-novel concepts, register a custom `com.<owner>.<name>` type.

- `text` — text
- `tags_csv` — text (optional, comma-separated; not normative for queries — for the user's organization)

---

## Custom event types

Any user, app, or vendor can register custom event types in their `com.<owner>.<name>` namespace at any time. The registration mechanism is the storage library's `registry.add_event_type()` / `add_channel()` API; see [`storage-format.md`](storage-format.md) "Channel registry."

Custom types **round-trip through export/import**. An OHD instance receiving an export with custom types it doesn't understand stores them verbatim and can re-export them; queries against unknown types return empty without error.

If a custom type proves broadly useful, the project promotes it to `std.*` via a versioned registry update; existing custom-type rows resolve through `type_aliases` to the new standard ID with no rewrite. See [`storage-format.md`](storage-format.md) "Migrations."

## What this doc deliberately does NOT contain

| | Where it lives |
|---|---|
| Wire format (Protobuf / JSON / encoding) | OHDC `.proto`; see [`../components/connect.md`](../components/connect.md) |
| On-disk SQL schema (events / event_channels / event_samples / etc.) | [`storage-format.md`](storage-format.md) |
| Sample-block compression encoding | [`storage-format.md`](storage-format.md) "Sample blocks" |
| Storage density numbers and capacity planning | [`storage-format.md`](storage-format.md) |
| Export/import file format | OHDC v1 protocol spec (Task #8) when it lands |
| Postgres schema | Was in this doc's v0; does not exist in the contracted architecture |

## Cross-references

- On-disk format and channel-tree mechanics: [`storage-format.md`](storage-format.md)
- Privacy / sensitivity-class semantics: [`privacy-access.md`](privacy-access.md)
- OHDC protocol (wire format): [`../components/connect.md`](../components/connect.md)
- Authentication and tokens: [`auth.md`](auth.md)
