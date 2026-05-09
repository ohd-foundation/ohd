// OHDC-backed store. Replaces the surface of `src/mock/store.ts` with calls
// into the OHDC client wrapper at `./client`.
//
// The mock store was sync — `listPatients()` returned a `PatientSummary[]`
// directly. We preserve that contract so the UI doesn't have to change. The
// underlying OHDC calls are async, so the store maintains an in-memory
// snapshot that is hydrated on `bootstrap()` and refreshed after every write.
//
// Subscribers (the `useStoreVersion` hook) re-render whenever the snapshot
// version increments. v0 has exactly one patient (the grant's owner) so the
// snapshot is small.

import type {
  ApprovalMode,
  ClinicalNote,
  FoodEntry,
  GrantStatus,
  ImagingStudy,
  LabResult,
  MedicationEntry,
  Operator,
  PatientDetail,
  PatientSummary,
  SymptomEntry,
  TimelineEvent,
  VitalReading,
} from "../types";
import {
  putEvent,
  queryEvents,
  resolveGrantToken,
  ulidToCrockford,
  whoAmI,
  type Event,
} from "./client";

// --- Constants --------------------------------------------------------------

/**
 * The single operator driving Care v0. Replaces the mock OPERATOR;
 * a real operator OIDC flow + session token is the v0.x deliverable.
 */
export const MOCK_OPERATOR: Operator = {
  display_name: "Dr. Smith",
  role: "clinician",
  status: "online",
};

/**
 * Slug used for the single v0 patient. Stable across reloads so URL routing
 * works after a refresh. (v0.x: derive from the grant ULID once we hold N
 * grants in the vault.)
 */
const PATIENT_SLUG = "patient";

/**
 * The five OHDC `std.*` event types we read for v0. Mirrors the Connect CLI's
 * `log` subcommand so the demo's seeded events show up.
 */
const READ_EVENT_TYPES = [
  "std.blood_glucose",
  "std.heart_rate_resting",
  "std.body_temperature",
  "std.medication_dose",
  "std.symptom",
  "std.clinical_note",
];

// --- Snapshot state ---------------------------------------------------------

interface Snapshot {
  /** v0: one patient. Empty until bootstrap completes. */
  patients: PatientDetail[];
  /** Bootstrap completed (regardless of whether it succeeded). */
  ready: boolean;
  /** Set when bootstrap fails (e.g. invalid grant). */
  error: string | null;
}

let snapshot: Snapshot = { patients: [], ready: false, error: null };

const subscribers = new Set<() => void>();
let version = 0;

function notify() {
  version += 1;
  for (const s of subscribers) s();
}

/** Subscribe to snapshot changes. Returns the unsubscribe function. */
export function subscribe(fn: () => void): () => void {
  subscribers.add(fn);
  return () => {
    subscribers.delete(fn);
  };
}

/** Read the current version counter. Cheap; bumps on every snapshot change. */
export function getVersion(): number {
  return version;
}

/** Read the bootstrap status. */
export function getBootstrapStatus(): { ready: boolean; error: string | null } {
  return { ready: snapshot.ready, error: snapshot.error };
}

// --- Bootstrap --------------------------------------------------------------

let bootstrapping: Promise<void> | null = null;

/**
 * Idempotent: call WhoAmI + the initial QueryEvents pass to populate the
 * snapshot. Subsequent calls return the same promise.
 *
 * If there's no token, the snapshot stays empty and `error` is "no_token".
 */
export function bootstrap(): Promise<void> {
  if (bootstrapping) return bootstrapping;
  bootstrapping = (async () => {
    const token = resolveGrantToken();
    if (!token) {
      snapshot = { patients: [], ready: true, error: "no_token" };
      notify();
      return;
    }
    try {
      const me = await whoAmI();
      if (!me) {
        snapshot = { patients: [], ready: true, error: "whoami_failed" };
        notify();
        return;
      }
      const events = await queryEvents({ eventTypes: READ_EVENT_TYPES, limit: 1000 });
      const detail = buildPatientDetail(me, events);
      snapshot = { patients: [detail], ready: true, error: null };
      notify();
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error("OHDC bootstrap failed", err);
      snapshot = {
        patients: [],
        ready: true,
        error: `bootstrap_failed: ${(err as Error).message ?? String(err)}`,
      };
      notify();
    }
  })();
  return bootstrapping;
}

/** Force a fresh fetch from OHDC. Used after writes. */
export async function refresh(): Promise<void> {
  const token = resolveGrantToken();
  if (!token) return;
  try {
    const me = await whoAmI();
    if (!me) return;
    const events = await queryEvents({ eventTypes: READ_EVENT_TYPES, limit: 1000 });
    const detail = buildPatientDetail(me, events);
    snapshot = { patients: [detail], ready: true, error: null };
    notify();
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error("OHDC refresh failed", err);
  }
}

// --- Public sync API (mirrors the mock store) ------------------------------

export function listPatients(): PatientSummary[] {
  return snapshot.patients.map((p) => ({
    label: p.label,
    slug: p.slug,
    display_name: p.display_name,
    last_visit_ms: p.last_visit_ms,
    flags: p.flags,
    meds_summary: p.meds_summary,
    grant: p.grant,
    active_case: p.active_case,
  }));
}

export function getPatientBySlug(slug: string): PatientDetail | undefined {
  return snapshot.patients.find((p) => p.slug === slug);
}

// --- Submission helpers ----------------------------------------------------

/**
 * Append a clinical note via `OhdcService.PutEvents` with `std.clinical_note`.
 *
 * Returns a synchronous "optimistic" `ClinicalNote` immediately — the network
 * call fires in the background and a `refresh()` follows. If the grant's
 * approval_mode is `always`, the note will land in `pending_events` and only
 * surface on a subsequent refresh after the patient approves.
 */
export function submitNote(slug: string, text: string, author: string): ClinicalNote | null {
  const p = getPatientBySlug(slug);
  if (!p) return null;
  const ts = Date.now();
  // Optimistic insert — the UI shows "pending_patient_approval" right away
  // even though the OHDC PutEvents response will land later.
  const optimistic: ClinicalNote = {
    ts_ms: ts,
    author,
    text,
    status: "pending_patient_approval",
  };
  p.notes.unshift(optimistic);
  p.timeline.unshift({
    ts_ms: ts,
    event_type: "note",
    summary: `Clinical note by ${author} [${optimistic.status}]`,
    detail: text,
  });
  notify();

  // Fire-and-forget; the result will be reconciled by `refresh()`.
  void (async () => {
    try {
      const outcome = await putEvent({
        eventType: "std.clinical_note",
        timestampMs: ts,
        notes: text,
        channels: [
          { channelPath: "text", value: { kind: "text", textValue: text } },
          { channelPath: "author", value: { kind: "text", textValue: author } },
        ],
      });
      // If the registry doesn't have `std.clinical_note` (or the grant doesn't
      // permit it), drop the optimistic note and surface the error.
      if (outcome.kind === "error") {
        // eslint-disable-next-line no-console
        console.error("submitNote OHDC error:", outcome.code, outcome.message);
      }
      void refresh();
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error("submitNote network error:", err);
    }
  })();

  return optimistic;
}

export function submitVital(
  slug: string,
  channel: string,
  value: number,
  unit: string,
): VitalReading | null {
  const p = getPatientBySlug(slug);
  if (!p) return null;
  const v: VitalReading = { ts_ms: Date.now(), channel, value, unit };
  p.vitals.push(v);
  p.timeline.unshift({
    ts_ms: v.ts_ms,
    event_type: "vital",
    summary: `${channel} = ${value} ${unit}`,
  });
  notify();

  // Map the channel selector to an OHDC event type. v0 supports the three
  // std.* types in our READ_EVENT_TYPES list; otherwise the write is dropped
  // and logged.
  const mapping = mapVitalChannelToEvent(channel, value);
  if (mapping) {
    void (async () => {
      try {
        await putEvent({
          eventType: mapping.eventType,
          timestampMs: v.ts_ms,
          channels: mapping.channels,
        });
        void refresh();
      } catch (err) {
        // eslint-disable-next-line no-console
        console.error("submitVital error:", err);
      }
    })();
  } else {
    // eslint-disable-next-line no-console
    console.warn(`submitVital: no OHDC mapping for channel ${channel}; optimistic only`);
  }
  return v;
}

export function submitSymptom(
  slug: string,
  text: string,
  severity: 1 | 2 | 3 | 4 | 5,
): SymptomEntry | null {
  const p = getPatientBySlug(slug);
  if (!p) return null;
  const s: SymptomEntry = { ts_ms: Date.now(), text, severity };
  p.symptoms.unshift(s);
  p.timeline.unshift({
    ts_ms: s.ts_ms,
    event_type: "symptom",
    summary: `Symptom (severity ${severity}): ${text}`,
  });
  notify();
  void (async () => {
    try {
      await putEvent({
        eventType: "std.symptom",
        timestampMs: s.ts_ms,
        channels: [
          { channelPath: "name", value: { kind: "text", textValue: text } },
          { channelPath: "severity", value: { kind: "int", intValue: severity } },
        ],
      });
      void refresh();
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error("submitSymptom error:", err);
    }
  })();
  return s;
}

export function submitFood(
  slug: string,
  text: string,
  kcal?: number,
): FoodEntry | null {
  const p = getPatientBySlug(slug);
  if (!p) return null;
  const f: FoodEntry = { ts_ms: Date.now(), text, kcal };
  p.foods.unshift(f);
  p.timeline.unshift({
    ts_ms: f.ts_ms,
    event_type: "food",
    summary: `Food: ${text}${kcal ? ` (~${kcal} kcal)` : ""}`,
  });
  notify();
  // No std.meal write yet — the registry seeds `std.meal` but the demo
  // grant's write-scope is just `std.clinical_note`. Optimistic only.
  // eslint-disable-next-line no-console
  console.info("submitFood: optimistic only (std.meal write deferred to v0.x)");
  return f;
}

export function submitMedication(
  slug: string,
  name: string,
  dose: string,
  schedule: string,
): MedicationEntry | null {
  const p = getPatientBySlug(slug);
  if (!p) return null;
  const m: MedicationEntry = {
    name,
    dose,
    schedule,
    recent_doses: [],
    active: true,
  };
  p.medications.unshift(m);
  p.timeline.unshift({
    ts_ms: Date.now(),
    event_type: "medication",
    summary: `Prescribed ${name} ${dose} (${schedule}) [pending_patient_approval]`,
  });
  notify();
  // std.medication_dose maps to a "taken" event, not a "prescription". A
  // dedicated `std.medication_prescription` event type is on the v0.x list.
  // eslint-disable-next-line no-console
  console.info("submitMedication: optimistic only (no std.medication_prescription yet)");
  return m;
}

export function submitLab(
  slug: string,
  panel: string,
  valuesText: string,
): LabResult | null {
  const p = getPatientBySlug(slug);
  if (!p) return null;
  const lab: LabResult = {
    ts_ms: Date.now(),
    panel,
    values: [{ name: "Result", value: valuesText, flag: "normal" }],
  };
  p.labs.unshift(lab);
  p.timeline.unshift({
    ts_ms: lab.ts_ms,
    event_type: "lab",
    summary: `Lab panel: ${panel} [pending_patient_approval]`,
  });
  notify();
  // No std.lab_result write yet — registry doesn't seed it for v0.
  // eslint-disable-next-line no-console
  console.info("submitLab: optimistic only (std.lab_result deferred to v0.x)");
  return lab;
}

export function submitImaging(
  slug: string,
  modality: string,
  region: string,
  findings: string,
): ImagingStudy | null {
  const p = getPatientBySlug(slug);
  if (!p) return null;
  const i: ImagingStudy = { ts_ms: Date.now(), modality, region, findings };
  p.imaging.unshift(i);
  p.timeline.unshift({
    ts_ms: i.ts_ms,
    event_type: "imaging",
    summary: `${modality} ${region} [pending_patient_approval]`,
    detail: findings,
  });
  notify();
  // eslint-disable-next-line no-console
  console.info("submitImaging: optimistic only (std.imaging_study deferred to v0.x)");
  return i;
}

// --- Internals --------------------------------------------------------------

/**
 * Build a single PatientDetail from WhoAmI + a flat list of events.
 *
 * The label is taken from the grant's `grantee_label` (Care side: "the
 * person whose grant we hold"), or the user_ulid as a fallback. The
 * grant's `read_scope` / `write_scope` summaries come from the grant's
 * effective rules; we surface them as event-type strings.
 */
function buildPatientDetail(
  me: { user_ulid?: { bytes: Uint8Array } | undefined; grantee_label?: string; effective_grant?: { approvalMode?: string; expiresAtMs?: bigint; eventTypeRules?: Array<{ eventType: string; effect: string }>; writeEventTypeRules?: Array<{ eventType: string; effect: string }>; } | undefined; tokenKind?: string },
  events: Event[],
): PatientDetail {
  // The patient label is the grant's grantee_label flipped: from Care's
  // perspective, the patient is the grant's *issuer*. The wire doesn't
  // distinguish, so v0 just uses the grantee_label as a stand-in plus the
  // user_ulid suffix.
  const userUlid = me.user_ulid?.bytes ? ulidToCrockford(me.user_ulid.bytes) : "<unknown>";
  const labelCore = me.grantee_label || userUlid.slice(0, 6);
  const label = `Patient ${labelCore} (${userUlid.slice(0, 8)})`;

  const grant = me.effective_grant ?? null;
  const approvalMode: ApprovalMode = (grant?.approvalMode as ApprovalMode) || "always";
  const expiresAtMs = grant?.expiresAtMs ? Number(grant.expiresAtMs) : null;
  const status: GrantStatus =
    expiresAtMs && expiresAtMs - Date.now() < 7 * 86_400_000 ? "expiring_soon" : "active";

  // Read-scope / write-scope summaries: collapse the rule list into the
  // `allow`-effect event types only. Empty grant + closed default = empty list.
  const readScope = (grant?.eventTypeRules ?? [])
    .filter((r: { effect: string }) => r.effect === "allow")
    .map((r: { eventType: string }) => r.eventType);
  const writeScope = (grant?.writeEventTypeRules ?? [])
    .filter((r: { effect: string }) => r.effect === "allow")
    .map((r: { eventType: string }) => r.eventType);

  const lastVisitMs = events.length > 0 ? Math.max(...events.map((e) => Number(e.timestampMs))) : null;

  // Project events into the per-tab buckets.
  const vitals: VitalReading[] = [];
  const medications: MedicationEntry[] = [];
  const symptoms: SymptomEntry[] = [];
  const foods: FoodEntry[] = [];
  const labs: LabResult[] = [];
  const imaging: ImagingStudy[] = [];
  const notes: ClinicalNote[] = [];

  // Group medication doses by name to emit one MedicationEntry per name.
  const medByName = new Map<string, MedicationEntry>();

  for (const e of events) {
    const ts = Number(e.timestampMs);
    switch (e.eventType) {
      case "std.blood_glucose":
        vitals.push({
          ts_ms: ts,
          channel: "glucose_mg_dl",
          value: readReal(e, "value") ?? 0,
          unit: "mg/dL",
        });
        break;
      case "std.heart_rate_resting":
        vitals.push({
          ts_ms: ts,
          channel: "hr",
          value: readReal(e, "bpm") ?? readReal(e, "value") ?? 0,
          unit: "bpm",
        });
        break;
      case "std.body_temperature":
        vitals.push({
          ts_ms: ts,
          channel: "temp_c",
          value: readReal(e, "value") ?? 0,
          unit: "°C",
        });
        break;
      case "std.medication_dose": {
        const name = readText(e, "name") ?? "(unknown med)";
        const dose = readReal(e, "dose");
        const doseUnit = readText(e, "dose_unit") ?? "";
        const status = readText(e, "status") ?? "taken";
        let entry = medByName.get(name);
        if (!entry) {
          entry = {
            name,
            dose: dose != null ? `${dose} ${doseUnit}`.trim() : doseUnit,
            schedule: "as logged",
            recent_doses: [],
            active: true,
          };
          medByName.set(name, entry);
        }
        entry.recent_doses.push({ ts_ms: ts, taken: status === "taken" });
        break;
      }
      case "std.symptom": {
        const name = readText(e, "name") ?? "(symptom)";
        const sev = readInt(e, "severity") ?? 3;
        // Clamp severity to the UI's 1–5 scale (CLI emits 0–10; we bucket).
        const ui_sev = Math.max(1, Math.min(5, Math.round((sev / 10) * 5))) as 1 | 2 | 3 | 4 | 5;
        symptoms.push({ ts_ms: ts, text: name, severity: ui_sev });
        break;
      }
      case "std.clinical_note": {
        notes.push({
          ts_ms: ts,
          author: readText(e, "author") ?? "?",
          text: readText(e, "text") ?? e.notes ?? "",
          status: "committed",
        });
        break;
      }
      default:
        // Unrecognized event type — skip silently. The full clinical-note +
        // food + lab + imaging mappings land alongside the registry seeds.
        break;
    }
  }
  for (const m of medByName.values()) {
    m.recent_doses.sort((a, b) => a.ts_ms - b.ts_ms);
    medications.push(m);
  }

  // Sort each per-tab list by timestamp DESC so the most recent is first.
  vitals.sort((a, b) => b.ts_ms - a.ts_ms);
  symptoms.sort((a, b) => b.ts_ms - a.ts_ms);
  notes.sort((a, b) => b.ts_ms - a.ts_ms);

  const flags: string[] = [];
  if (status === "expiring_soon") {
    flags.push("grant expiring soon");
  }
  if (events.length === 0) {
    flags.push("no events yet");
  }

  const meds_summary =
    medications.length === 0
      ? ["No medication events logged"]
      : medications.slice(0, 3).map((m) => `${m.name}${m.dose ? ` ${m.dose}` : ""}`);

  // Visit-prep brief: a few shallow bullets so the panel isn't empty.
  const brief: string[] = [];
  brief.push(`${events.length} event${events.length === 1 ? "" : "s"} loaded via grant.`);
  if (vitals.length) brief.push(`Vitals: ${vitals.length} readings.`);
  if (medications.length) brief.push(`Medications: ${medications.length} unique.`);
  if (symptoms.length) brief.push(`Symptoms: ${symptoms.length} logged.`);
  if (notes.length) brief.push(`Clinical notes: ${notes.length}.`);

  const timeline = buildTimeline({ vitals, medications, symptoms, foods, labs, imaging, notes });

  return {
    label,
    slug: PATIENT_SLUG,
    display_name: labelCore,
    last_visit_ms: lastVisitMs,
    flags,
    meds_summary,
    grant: {
      read_scope: readScope,
      write_scope: writeScope,
      approval_mode: approvalMode,
      expires_at_ms: expiresAtMs,
      status,
    },
    brief,
    vitals,
    medications,
    symptoms,
    foods,
    labs,
    imaging,
    notes,
    timeline,
  };
}

function readReal(e: Event, channelPath: string): number | null {
  for (const c of e.channels) {
    if (c.channelPath !== channelPath) continue;
    if (c.value.case === "realValue") return c.value.value;
    if (c.value.case === "intValue") return Number(c.value.value);
  }
  return null;
}

function readInt(e: Event, channelPath: string): number | null {
  for (const c of e.channels) {
    if (c.channelPath !== channelPath) continue;
    if (c.value.case === "intValue") return Number(c.value.value);
    if (c.value.case === "realValue") return Math.round(c.value.value);
  }
  return null;
}

function readText(e: Event, channelPath: string): string | null {
  for (const c of e.channels) {
    if (c.channelPath !== channelPath) continue;
    if (c.value.case === "textValue") return c.value.value;
    if (c.value.case === "enumOrdinal") return String(c.value.value);
  }
  return null;
}

function buildTimeline(p: {
  vitals: VitalReading[];
  medications: MedicationEntry[];
  symptoms: SymptomEntry[];
  foods: FoodEntry[];
  labs: LabResult[];
  imaging: ImagingStudy[];
  notes: ClinicalNote[];
}): TimelineEvent[] {
  const events: TimelineEvent[] = [];
  for (const v of p.vitals) {
    events.push({ ts_ms: v.ts_ms, event_type: "vital", summary: `${v.channel} = ${v.value} ${v.unit}` });
  }
  for (const s of p.symptoms) {
    events.push({
      ts_ms: s.ts_ms,
      event_type: "symptom",
      summary: `Symptom (severity ${s.severity}): ${s.text}`,
    });
  }
  for (const m of p.medications) {
    for (const d of m.recent_doses.slice(-5)) {
      events.push({
        ts_ms: d.ts_ms,
        event_type: "medication",
        summary: `${m.name} ${m.dose} — ${d.taken ? "taken" : "missed"}`,
      });
    }
  }
  for (const n of p.notes) {
    events.push({
      ts_ms: n.ts_ms,
      event_type: "note",
      summary: `Clinical note by ${n.author} [${n.status}]`,
      detail: n.text,
    });
  }
  return events.sort((a, b) => b.ts_ms - a.ts_ms);
}

interface VitalMapping {
  eventType: string;
  channels: Parameters<typeof putEvent>[0]["channels"];
}

/**
 * Map the legacy mock-store channel selector (`bp_systolic` / `hr` / etc.) to
 * the matching std.* event type + channel triple. Returns null when the
 * channel doesn't have an OHDC mapping yet.
 */
function mapVitalChannelToEvent(channel: string, value: number): VitalMapping | null {
  switch (channel) {
    case "hr":
      return {
        eventType: "std.heart_rate_resting",
        channels: [{ channelPath: "bpm", value: { kind: "real", realValue: value } }],
      };
    case "temp_c":
      return {
        eventType: "std.body_temperature",
        channels: [{ channelPath: "value", value: { kind: "real", realValue: value } }],
      };
    case "glucose_mg_dl":
      return {
        eventType: "std.blood_glucose",
        channels: [{ channelPath: "value", value: { kind: "real", realValue: value } }],
      };
    default:
      return null;
  }
}
