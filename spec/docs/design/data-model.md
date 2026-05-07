# Design: Data Model

> How we represent health data in OHD.
>
> **Note**: this document describes the *conceptual* event model — vocabulary, types, what each kind of event means. The *on-disk* format (tables, indexes, sample blocks, channel registry, migrations) lives in [`storage-format.md`](storage-format.md), which supersedes the Postgres schema and storage-density sections below for implementation purposes.

## Design goals

1. **Flexible.** Any health-relevant data a person might want to record should fit.
2. **Portable.** The schema defines an interchange format, not an implementation detail.
3. **Queryable.** Common queries (by time range, by event type) must be fast.
4. **Compact.** Store years of dense data without bloat, but *without lossy compression* in the general case.
5. **Extensible.** New event types don't require schema migrations.
6. **Auditable.** Every change is traceable.

## The core concept: everything is an event

Every piece of health data in OHD is an **event**. An event is something that happened at a point (or span) in time. Examples:

- A glucose reading at 14:32 is an event (instantaneous).
- Eating a meal from 18:00 to 18:45 is an event (with duration).
- Taking a pill at 07:13 is an event (instantaneous).
- A hospital stay from Monday to Friday is an event (with duration).
- Being diagnosed with hypertension in 2019 is an event (a point in time, even though the condition persists).
- A two-minute EKG recording is an event (with duration and an attached binary blob).

**Why "events" and not something more complex?** Because every health data type ultimately reduces to "something that happened to a person at a time." This is the simplest model that covers everything.

## Canonical event schema

```json
{
  "id": "01HF2K8XJQM4P5N7R9V3B2T6Y8",
  "user_id": "01HF2K8XJQM4P5N7R9V3B2T6Y7",
  "timestamp": "2025-01-15T14:32:18Z",
  "duration_seconds": null,
  "event_type": "glucose",
  "value": 6.4,
  "unit": "mmol/L",
  "data": {
    "measurement_method": "cgm",
    "device_id": "libre3_serial_xxx"
  },
  "metadata": {
    "source": "health_connect:com.librelinkup.app",
    "confidence": 1.0,
    "created_via": "ohdc_android_v0.1.0",
    "tags": []
  },
  "created_at": "2025-01-15T14:33:01Z",
  "updated_at": "2025-01-15T14:33:01Z",
  "schema_version": "1.0"
}
```

### Required fields

| Field | Type | Meaning |
|---|---|---|
| `id` | UUID / ULID | Globally unique event ID |
| `user_id` | UUID | The user this event belongs to |
| `timestamp` | ISO 8601 (UTC) | When the measured event occurred |
| `event_type` | string (from vocabulary) | What kind of event this is |
| `created_at` | ISO 8601 | When the event was recorded in OHD |
| `schema_version` | string | The protocol version of this event |

### Optional fields

| Field | Type | Meaning |
|---|---|---|
| `duration_seconds` | integer | For events with duration; null means instantaneous |
| `value` | number | The primary measurement, if any |
| `unit` | string | Unit of `value` |
| `data` | object | Event-type-specific structured fields |
| `metadata` | object | Source, confidence, provenance, tags, etc. |
| `updated_at` | ISO 8601 | Last modification time |

### Identifiers

We use **ULID** (`01HF2K8XJQM4P5N7R9V3B2T6Y8`) rather than UUIDv4. ULIDs are lexicographically sortable by creation time, which means indexes on `id` are naturally ordered and list scans are efficient. Postgres handles them fine as `CHAR(26)` or converted to UUID via ordered encoding.

### Timestamps

- Always UTC in storage and on the wire.
- Always ISO 8601 in JSON.
- Client apps display in the user's local timezone.
- `timestamp` is *when the measurement or event happened*. `created_at` is *when OHD received it*. These may differ by seconds (sync lag) or days (backfill).

## Event type vocabulary

The protocol defines a standard vocabulary. Connectors should use these types. Custom types are allowed (see "Extension" below) but using standard types when possible maximizes interoperability.

### Biometric measurements (instantaneous)

| Type | Typical unit | Notes |
|---|---|---|
| `glucose` | mmol/L or mg/dL | `data.measurement_method` = `cgm`/`fingerstick`/`lab` |
| `heart_rate` | bpm | |
| `blood_pressure_systolic` | mmHg | Usually logged together with diastolic |
| `blood_pressure_diastolic` | mmHg | |
| `body_temperature` | °C or °F | |
| `oxygen_saturation` | % | SpO2 |
| `respiratory_rate` | breaths/min | |
| `weight` | kg or lb | |
| `body_fat_percent` | % | |
| `muscle_mass` | kg | |
| `bone_mass` | kg | |
| `hydration_percent` | % | |
| `hrv` | ms | Heart rate variability |
| `peak_flow` | L/min | For asthma tracking |
| `blood_ketones` | mmol/L | |
| `urine_marker` | (varies) | `data.marker` = `glucose`/`protein`/`ketones`/... |

### Biometric measurements (continuous, aggregated)

| Type | Typical unit | Notes |
|---|---|---|
| `heart_rate_series` | bpm | `data.samples` = array of `{t, v}`, `duration_seconds` set |
| `glucose_series` | mmol/L | Same pattern |
| `ecg_recording` | µV | `data.blob_ref` = attachment ID, `duration_seconds` set |

### Events with duration

| Type | Notes |
|---|---|
| `sleep` | `data.stages` = array of `{start, end, stage}` |
| `exercise` | `data.activity`, `data.distance`, `data.calories` |
| `meal` | See "Food" below |
| `medication_dose` | See "Medications" below |
| `hospital_stay` | `data.admission_reason`, `data.discharge_summary` |

### Food events (`meal` or `food_item`)

```json
{
  "event_type": "meal",
  "timestamp": "2025-01-15T18:00:00Z",
  "duration_seconds": 2700,
  "data": {
    "items": [
      {
        "openfoodfacts_id": "3017620422003",
        "name": "Nutella",
        "quantity_grams": 30,
        "nutrition": {
          "energy_kcal": 161,
          "fat_g": 9.3,
          "saturated_fat_g": 3.2,
          "carbohydrates_g": 17.3,
          "sugars_g": 16.8,
          "fiber_g": 0,
          "proteins_g": 1.8,
          "salt_g": 0.03
        }
      }
    ],
    "total_nutrition": { "energy_kcal": 161, "..." : "..." },
    "notes": "with toast"
  }
}
```

### Medication events

Two related types:

**`medication_prescribed`** — a medication is prescribed / added to the user's list.
```json
{
  "event_type": "medication_prescribed",
  "timestamp": "2025-01-10T10:00:00Z",
  "data": {
    "medication_id": "user_defined_id_or_rxnorm",
    "name": "metformin",
    "dose": 500,
    "dose_unit": "mg",
    "schedule": "twice daily with meals",
    "prescribed_by": "Dr. Smith",
    "reason": "type 2 diabetes"
  }
}
```

**`medication_dose`** — the user actually took (or skipped) a dose.
```json
{
  "event_type": "medication_dose",
  "timestamp": "2025-01-15T07:13:00Z",
  "data": {
    "medication_id": "ref to prescribed event",
    "name": "metformin",
    "dose": 500,
    "dose_unit": "mg",
    "status": "taken",
    "notes": "with breakfast"
  }
}
```

Status can be `taken`, `skipped`, `late`, `refused`.

### Symptom events

```json
{
  "event_type": "symptom",
  "timestamp": "2025-01-15T09:00:00Z",
  "duration_seconds": null,
  "data": {
    "symptom": "headache",
    "severity": "moderate",
    "severity_scale": "1-10:5",
    "location": "frontal",
    "notes": "worse with bright light"
  }
}
```

### Medical records

| Type | Notes |
|---|---|
| `diagnosis` | `data.icd10_code`, `data.condition_name`, `data.diagnosed_by` |
| `lab_result` | `data.test_name`, `data.loinc_code`, `data.reference_range`, value in `value`/`unit` |
| `imaging` | `data.modality` (MRI/CT/X-ray), `data.report`, `data.blob_ref` for image |
| `procedure` | `data.procedure_name`, `data.cpt_code`, `data.performed_by` |
| `vaccination` | `data.vaccine_name`, `data.dose_number`, `data.lot_number` |
| `allergy` | `data.allergen`, `data.severity`, `data.reaction` |
| `consultation_note` | `data.provider`, `data.text` |

### Lifestyle / context

| Type | Notes |
|---|---|
| `hydration` | `value` = ml of water/beverage |
| `alcohol` | `data.beverage`, `value` = grams of ethanol |
| `caffeine` | `value` = mg of caffeine |
| `mood` | `data.mood`, `data.energy`, `data.notes` |
| `substance_use` | `data.substance`, `value` = dose |
| `menstrual_flow` | `data.flow` = none/light/normal/heavy |

### Generic fallback

| Type | Notes |
|---|---|
| `custom` | `data.custom_type` describes the user-defined type. Use sparingly; prefer standard types. |

## Extensions

The vocabulary is not exhaustive. Any Connector can add event types by using a namespaced identifier:

```
event_type: "com.mycompany.sleep_apnea_event"
```

Or by using the `custom` type with a user-defined sub-type:

```json
{
  "event_type": "custom",
  "data": {
    "custom_type": "post_surgery_wound_check",
    "attributes": {
      "redness": "mild",
      "swelling": "none",
      "pain": 3
    }
  }
}
```

**Portability rule:** Extensions must be preserved through export/import. An OHD instance that doesn't understand a custom type must still round-trip the raw data.

**Contribution rule:** If an extension is broadly useful, contribute it upstream and it becomes part of the standard vocabulary.

## Postgres schema (reference implementation)

```sql
CREATE EXTENSION IF NOT EXISTS "pgcrypto";

CREATE TABLE users (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    oidc_provider TEXT NOT NULL,
    oidc_subject TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (oidc_provider, oidc_subject)
);

CREATE TABLE health_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    timestamp TIMESTAMPTZ NOT NULL,
    duration_seconds INTEGER,
    event_type TEXT NOT NULL,
    value NUMERIC,
    unit TEXT,
    data JSONB NOT NULL DEFAULT '{}'::jsonb,
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    schema_version TEXT NOT NULL DEFAULT '1.0',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    deleted_at TIMESTAMPTZ  -- soft delete
);

CREATE INDEX idx_events_user_time
    ON health_events (user_id, timestamp DESC)
    WHERE deleted_at IS NULL;

CREATE INDEX idx_events_user_type_time
    ON health_events (user_id, event_type, timestamp DESC)
    WHERE deleted_at IS NULL;

-- Useful for source-based deduplication
CREATE INDEX idx_events_source_hash
    ON health_events ((metadata->>'source'), (metadata->>'source_id'))
    WHERE metadata ? 'source_id';

-- JSONB indexes are added later if specific queries need them.
-- Start without them to keep writes cheap.
```

Plus tables for grants, audit log, attachments (blobs for EKGs etc.), and OIDC sessions — detailed in `../design/privacy-access.md`.

## Storage density

Rough numbers for reasonable sampling rates:

| Data type | Rate | Raw size/day | After Postgres overhead |
|---|---|---|---|
| Glucose (CGM) | 1/20s = 4,320/day | ~17 KB as floats | ~200 KB with row metadata |
| Heart rate | 1/3s = 28,800/day | ~115 KB | ~2 MB |
| Steps / activity aggregate | ~100/day | ~4 KB | ~50 KB |
| Sleep stages | ~100/night | ~4 KB | ~50 KB |
| Meals | ~5/day | ~5 KB (with nutrition) | ~50 KB |
| Medications | ~10/day | trivial | trivial |
| Manual entries | ~20/day | trivial | trivial |

**Per-user per-day total: ~2.5 MB uncompressed in Postgres.**

Per year: ~900 MB. Per decade: ~9 GB. Per lifetime (80 years): ~72 GB.

**Per-row overhead would dominate at these sampling rates** if we stored one DB row per sample. Instead, dense continuous series (heart rate, glucose, ECG) are stored as **sample blocks** — one row per ~15-minute window, with the samples as a compressed binary blob inside that row. See [`storage-format.md`](storage-format.md) "Sample blocks" for the on-disk encoding.

### Series events — illustration

Instead of ~28,800 rows per day for heart rate, one row per 15-minute window (96/day) holds 900 seconds of samples as a compressed `(t_offset, value)` stream:

```json
{
  "event_type": "heart_rate_series",
  "timestamp": "2025-01-15T14:00:00Z",
  "duration_seconds": 900,
  "data": {
    "sampling_interval_seconds": 3,
    "samples": [72, 73, 74, 72, ..., 78]
  }
}
```

- ~300 rows/day instead of ~30,000.
- The compressed sample blocks deliver ≥100× density gain over per-sample rows.
- `AVG(heart_rate) over last week` decodes block by block; latency stays well under what's needed for interactive use.
- Round-trippable to per-sample events for users or analyses that want them.

## Export/import format

The portable export is a JSON file with this structure:

```json
{
  "ohd_export": {
    "schema_version": "1.0",
    "exported_at": "2025-01-15T12:00:00Z",
    "exporter": "ohd-core v0.1.0",
    "user_id_opaque": "hash-of-original-user-id",
    "extensions_used": ["com.mycompany.sleep_apnea_event"],
    "events": [
      { /* event objects as above */ }
    ],
    "attachments": [
      {
        "id": "...",
        "event_id": "...",
        "mime_type": "application/octet-stream",
        "content_base64": "..."
      }
    ],
    "grants_historical": [
      { /* grant records, for audit completeness */ }
    ],
    "audit_log": [
      { /* audit entries, for accountability */ }
    ]
  }
}
```

For large exports, the format also supports streaming / chunked variants (NDJSON), but the signed root manifest structure is the same.

**Signatures.** The export is signed by the source OHD instance's key. Importing instances verify the signature (if they trust the source) or surface a warning (if they don't).

**Lossy-but-portable rule.** If an extension can't be represented in the target instance, it's moved to `metadata._imported_extensions` as raw JSON and preserved through any re-export. No silent data loss.

## Open questions

- **Time-zone annotations.** Should we store the original timezone alongside UTC? Probably yes — "I ate dinner at 19:00 local" is different information from "I ate dinner at 03:00 UTC while on an intercontinental flight."
- **Measurement uncertainty.** Some readings have known error bars. Should we represent them? Proposal: `metadata.uncertainty = { method: "±5%", range: [value-error, value+error] }`. Optional, not required.
- **Immutability of historical data.** If a user edits an event, do we version it (keep both)? Probably yes — with `previous_version_id` links. Edits are rare; storage cost is low.
- **Binary attachments (EKGs, images).** Stored separately in object storage, referenced by `data.blob_ref`. Not in the event row. Exported as base64 for portability.
