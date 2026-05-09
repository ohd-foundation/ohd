// In-memory mock store fallback.
//
// **Used only when `VITE_USE_MOCK_STORE=1` is set at build time** — the
// canonical store (`./store.ts`) re-exports `../ohdc/store`. This file
// preserves the original 5-patient mock data so UI layouts can be
// exercised offline without a running storage server.
//
// No backend, no localStorage. Reset on page reload.
//
// 5 patients with varied state per the SPEC §3.1 contract:
//   - one with active flags (BP trending up, missed doses) — "Alice"
//   - one with no recent activity — "Pavel"
//   - one with current case open (EMS handoff) — "Marta"
//   - one with patient-curated case grant — "Jiri"
//   - one with a vanilla active grant — "Eva"

import type {
  ClinicalNote,
  FoodEntry,
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

const NOW = Date.now();
const HOUR = 60 * 60 * 1000;
const DAY = 24 * HOUR;

function slugify(label: string): string {
  return label
    .toLowerCase()
    .replace(/[^\w\s-]/g, "")
    .trim()
    .replace(/\s+/g, "-");
}

// --- The mock operator -------------------------------------------------------

export const MOCK_OPERATOR: Operator = {
  display_name: "Dr. Smith",
  role: "clinician",
  status: "online",
};

// --- Patient data ------------------------------------------------------------

interface SeedPatient extends Omit<PatientDetail, "slug"> {}

function seedAlice(): SeedPatient {
  // Active flags: BP trending up, missed doses.
  const bpReadings: VitalReading[] = Array.from({ length: 14 }).map((_, i) => ({
    ts_ms: NOW - (13 - i) * DAY,
    channel: "bp_systolic",
    value: 128 + i * 1.2 + (i % 3 === 0 ? 4 : 0),
    unit: "mmHg",
  }));
  const hrReadings: VitalReading[] = Array.from({ length: 14 }).map((_, i) => ({
    ts_ms: NOW - (13 - i) * DAY,
    channel: "hr",
    value: 72 + (i % 4),
    unit: "bpm",
  }));
  const metforminDoses = Array.from({ length: 14 }).map((_, i) => ({
    ts_ms: NOW - (13 - i) * DAY - 8 * HOUR,
    taken: ![2, 5, 9, 12].includes(i),
  }));

  return {
    label: "Alice (DOB 1985-04-12)",
    display_name: "Alice N.",
    last_visit_ms: NOW - 5 * DAY,
    flags: ["BP trending up over 14d", "missed 4 of last 14 metformin doses", "new symptom in last 48h"],
    meds_summary: [
      "Metformin 500 mg — 2× daily",
      "Lisinopril 10 mg — 1× daily",
      "Atorvastatin 20 mg — at bedtime",
    ],
    grant: {
      read_scope: ["vitals", "medications", "symptoms", "foods", "labs"],
      write_scope: ["clinical_note", "lab_result", "referral"],
      approval_mode: "auto_for_event_types",
      expires_at_ms: NOW + 60 * DAY,
      status: "active",
    },
    brief: [
      "BP systolic trending up over last 14d (avg 138, peak 152 yesterday).",
      "Missed 4 of last 14 metformin doses — adherence 71%.",
      "New symptom logged 36h ago: 'lightheaded on standing'.",
      "Last lab panel 28d ago — A1c 7.2, LDL 118.",
    ],
    vitals: [...bpReadings, ...hrReadings],
    medications: [
      {
        name: "Metformin",
        dose: "500 mg",
        schedule: "2× daily with meals",
        recent_doses: metforminDoses,
        active: true,
      },
      {
        name: "Lisinopril",
        dose: "10 mg",
        schedule: "1× daily, morning",
        recent_doses: Array.from({ length: 14 }).map((_, i) => ({
          ts_ms: NOW - (13 - i) * DAY - 7 * HOUR,
          taken: true,
        })),
        active: true,
      },
      {
        name: "Atorvastatin",
        dose: "20 mg",
        schedule: "at bedtime",
        recent_doses: Array.from({ length: 14 }).map((_, i) => ({
          ts_ms: NOW - (13 - i) * DAY - 22 * HOUR,
          taken: i !== 7,
        })),
        active: true,
      },
    ],
    symptoms: [
      { ts_ms: NOW - 36 * HOUR, text: "Lightheaded on standing, ~5s", severity: 3 },
      { ts_ms: NOW - 4 * DAY, text: "Mild headache, mostly evening", severity: 2 },
      { ts_ms: NOW - 9 * DAY, text: "Tingling left foot", severity: 2 },
    ],
    foods: [
      { ts_ms: NOW - 4 * HOUR, text: "Oatmeal w/ blueberries, coffee", kcal: 320 },
      { ts_ms: NOW - 22 * HOUR, text: "Chicken salad sandwich", kcal: 540 },
      { ts_ms: NOW - 28 * HOUR, text: "Pasta carbonara, white wine", kcal: 780 },
    ],
    labs: [
      {
        ts_ms: NOW - 28 * DAY,
        panel: "Metabolic + lipid + A1c",
        values: [
          { name: "A1c", value: "7.2", range: "4.0–5.6", flag: "high" },
          { name: "Glucose (fasting)", value: "138", range: "70–99", flag: "high" },
          { name: "LDL", value: "118", range: "<100", flag: "high" },
          { name: "HDL", value: "52", range: ">40", flag: "normal" },
          { name: "Creatinine", value: "0.9", range: "0.6–1.2", flag: "normal" },
        ],
      },
    ],
    imaging: [
      {
        ts_ms: NOW - 92 * DAY,
        modality: "X-ray",
        region: "Chest PA/lat",
        findings: "No acute cardiopulmonary findings. Cardiomediastinal silhouette normal.",
      },
    ],
    notes: [
      {
        ts_ms: NOW - 5 * DAY,
        author: "Dr. Smith",
        text:
          "Followup for T2DM + HTN. BP slightly elevated today. Reinforced med adherence; reviewed home glucose log.",
        status: "committed",
      },
      {
        ts_ms: NOW - 35 * DAY,
        author: "Dr. Smith",
        text: "Annual physical. Labs ordered. Patient asks about reducing statin dose.",
        status: "committed",
      },
    ],
    timeline: [],
  };
}

function seedPavel(): SeedPatient {
  // No recent activity — quiet patient, grant expiring soon.
  return {
    label: "Pavel K. (room 3B)",
    display_name: "Pavel K.",
    last_visit_ms: NOW - 180 * DAY,
    flags: ["grant expires in 6 days"],
    meds_summary: ["No active medications"],
    grant: {
      read_scope: ["vitals", "medications"],
      write_scope: [],
      approval_mode: "always",
      expires_at_ms: NOW + 6 * DAY,
      status: "expiring_soon",
    },
    brief: [
      "Last visit 6 months ago — annual physical.",
      "No recent events. Grant expires in 6 days.",
      "No active medications.",
    ],
    vitals: [],
    medications: [],
    symptoms: [],
    foods: [],
    labs: [
      {
        ts_ms: NOW - 180 * DAY,
        panel: "Annual basic panel",
        values: [
          { name: "Creatinine", value: "0.8", range: "0.6–1.2", flag: "normal" },
          { name: "ALT", value: "22", range: "7–56", flag: "normal" },
        ],
      },
    ],
    imaging: [],
    notes: [
      {
        ts_ms: NOW - 180 * DAY,
        author: "Dr. Smith",
        text: "Annual physical, all unremarkable. Return in 12 months.",
        status: "committed",
      },
    ],
    timeline: [],
  };
}

function seedMarta(): SeedPatient {
  // Active case: EMS Prague Region — open since 14:23.
  const today14_23 = new Date();
  today14_23.setHours(14, 23, 0, 0);
  const caseStarted = today14_23.getTime();

  const hrReadings: VitalReading[] = Array.from({ length: 6 }).map((_, i) => ({
    ts_ms: caseStarted + i * 10 * 60 * 1000,
    channel: "hr",
    value: 110 - i * 3,
    unit: "bpm",
  }));
  const bpReadings: VitalReading[] = Array.from({ length: 6 }).map((_, i) => ({
    ts_ms: caseStarted + i * 10 * 60 * 1000,
    channel: "bp_systolic",
    value: 95 + i * 2,
    unit: "mmHg",
  }));

  return {
    label: "Marta V. — EMS handoff",
    display_name: "Marta V.",
    last_visit_ms: caseStarted,
    flags: ["EMS handoff in progress", "tachycardic on arrival"],
    meds_summary: ["IV fluids running (NS @125 mL/h, started 14:30)"],
    grant: {
      read_scope: ["vitals", "medications", "symptoms"],
      write_scope: ["clinical_note", "observation", "medication_administered"],
      approval_mode: "never_required",
      expires_at_ms: caseStarted + 24 * HOUR,
      status: "case_bound",
      case_label: "EMS Prague Region — handoff",
      case_event_count: 14,
    },
    active_case: {
      label: "EMS Prague Region — handoff",
      authority: "EMS Prague Region Unit 12",
      started_ms: caseStarted,
    },
    brief: [
      "Case open since 14:23 (predecessor: EMS Prague Region Unit 12).",
      "HR 110 → 95 over last 60 min; BP 95/62 → 105/70.",
      "Chief complaint: collapse + chest pain on scene.",
      "EMS gave 250 mL NS bolus + 4 mg Zofran en route.",
    ],
    vitals: [...hrReadings, ...bpReadings],
    medications: [
      {
        name: "Normal saline IV",
        dose: "125 mL/h",
        schedule: "continuous",
        recent_doses: [{ ts_ms: caseStarted + 7 * 60 * 1000, taken: true }],
        active: true,
      },
      {
        name: "Ondansetron (EMS)",
        dose: "4 mg IV",
        schedule: "single dose en route",
        recent_doses: [{ ts_ms: caseStarted - 12 * 60 * 1000, taken: true }],
        active: false,
      },
    ],
    symptoms: [
      { ts_ms: caseStarted - 18 * 60 * 1000, text: "Chest pain, substernal, 6/10", severity: 4 },
      { ts_ms: caseStarted - 30 * 60 * 1000, text: "Brief LOC at home (per family)", severity: 5 },
    ],
    foods: [],
    labs: [],
    imaging: [],
    notes: [
      {
        ts_ms: caseStarted + 5 * 60 * 1000,
        author: "Dr. Smith",
        text: "Patient arrived via EMS; predecessor case open. Initial assessment in progress.",
        status: "auto_committed",
      },
    ],
    timeline: [],
  };
}

function seedJiri(): SeedPatient {
  // Patient-curated case grant — "Headaches — visit Dr. Smith 2026-05-15"
  return {
    label: "Jiri M. — curated visit",
    display_name: "Jiri M.",
    last_visit_ms: NOW - 60 * DAY,
    flags: ["Curated visit — 18 events linked"],
    meds_summary: ["Sumatriptan 50 mg PRN", "Magnesium 400 mg daily (self-started)"],
    grant: {
      read_scope: ["case-curated subset"],
      write_scope: ["clinical_note"],
      approval_mode: "always",
      expires_at_ms: NOW + 7 * DAY,
      status: "case_bound",
      case_label: "Headaches — visit Dr. Smith 2026-05-15",
      case_event_count: 18,
    },
    brief: [
      "Patient-curated case: 7 headache symptom logs, 6 sleep entries, 5 BP readings.",
      "Migraine pattern: aura → throbbing right-sided pain, 4–8h duration.",
      "Tried sumatriptan 50 mg — partial relief 4 of 7 events.",
      "Adopted magnesium 400 mg/day 3 weeks ago — patient reports fewer events.",
    ],
    vitals: Array.from({ length: 5 }).map((_, i) => ({
      ts_ms: NOW - (28 - i * 5) * DAY,
      channel: "bp_systolic",
      value: 122 + (i % 2) * 4,
      unit: "mmHg",
    })),
    medications: [
      {
        name: "Sumatriptan",
        dose: "50 mg",
        schedule: "PRN — at headache onset",
        recent_doses: [
          { ts_ms: NOW - 4 * DAY, taken: true },
          { ts_ms: NOW - 11 * DAY, taken: true },
          { ts_ms: NOW - 18 * DAY, taken: true },
        ],
        active: true,
      },
      {
        name: "Magnesium",
        dose: "400 mg",
        schedule: "1× daily",
        recent_doses: Array.from({ length: 14 }).map((_, i) => ({
          ts_ms: NOW - (13 - i) * DAY - 8 * HOUR,
          taken: i !== 4,
        })),
        active: true,
      },
    ],
    symptoms: Array.from({ length: 7 }).map((_, i) => ({
      ts_ms: NOW - (4 + i * 4) * DAY,
      text: `Headache — right-sided throbbing, ${4 + (i % 3)}h duration`,
      severity: ((4 + (i % 2)) as 4 | 5),
    })),
    foods: [],
    labs: [],
    imaging: [],
    notes: [
      {
        ts_ms: NOW - 60 * DAY,
        author: "Dr. Smith",
        text: "Initial migraine workup. Trial of sumatriptan PRN. Consider preventive if frequency increases.",
        status: "committed",
      },
    ],
    timeline: [],
  };
}

function seedEva(): SeedPatient {
  // Vanilla active grant, asthma + recent ER visit.
  return {
    label: "Eva R. (DOB 1992-09-03)",
    display_name: "Eva R.",
    last_visit_ms: NOW - 21 * DAY,
    flags: ["asthma flare 9d ago", "ICS adherence 100%"],
    meds_summary: [
      "Fluticasone inhaler 110 µg — 2 puffs BID",
      "Albuterol PRN — last used 9d ago",
    ],
    grant: {
      read_scope: ["vitals", "medications", "symptoms", "foods"],
      write_scope: ["clinical_note", "lab_result"],
      approval_mode: "always",
      expires_at_ms: NOW + 120 * DAY,
      status: "active",
    },
    brief: [
      "Asthma flare 9d ago — used albuterol; resolved within 24h.",
      "ICS (fluticasone) adherence 100% over last 14d.",
      "No ER visit since 2025; PEFR stable.",
      "Last spirometry 21d ago: FEV1 92% predicted.",
    ],
    vitals: Array.from({ length: 14 }).map((_, i) => ({
      ts_ms: NOW - (13 - i) * DAY,
      channel: "spo2",
      value: 97 + (i % 2),
      unit: "%",
    })),
    medications: [
      {
        name: "Fluticasone (ICS)",
        dose: "110 µg",
        schedule: "2 puffs BID",
        recent_doses: Array.from({ length: 28 }).map((_, i) => ({
          ts_ms: NOW - Math.floor((27 - i) / 2) * DAY - (i % 2 === 0 ? 8 : 20) * HOUR,
          taken: true,
        })),
        active: true,
      },
      {
        name: "Albuterol",
        dose: "90 µg",
        schedule: "PRN",
        recent_doses: [{ ts_ms: NOW - 9 * DAY, taken: true }],
        active: true,
      },
    ],
    symptoms: [
      { ts_ms: NOW - 9 * DAY, text: "Wheezing, used albuterol — relief in 10 min", severity: 3 },
    ],
    foods: [],
    labs: [
      {
        ts_ms: NOW - 21 * DAY,
        panel: "Spirometry",
        values: [
          { name: "FEV1", value: "92% predicted", flag: "normal" },
          { name: "FEV1/FVC", value: "0.81", flag: "normal" },
        ],
      },
    ],
    imaging: [],
    notes: [
      {
        ts_ms: NOW - 21 * DAY,
        author: "Dr. Smith",
        text: "Asthma well-controlled on ICS. Continue current regimen; review in 3 months.",
        status: "committed",
      },
    ],
    timeline: [],
  };
}

// --- Build the registry ------------------------------------------------------

function buildTimeline(p: Omit<PatientDetail, "slug" | "timeline">): TimelineEvent[] {
  const events: TimelineEvent[] = [];
  for (const v of p.vitals) {
    events.push({
      ts_ms: v.ts_ms,
      event_type: "vital",
      summary: `${v.channel} = ${v.value} ${v.unit}`,
    });
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
  for (const f of p.foods) {
    events.push({
      ts_ms: f.ts_ms,
      event_type: "food",
      summary: `Food: ${f.text}${f.kcal ? ` (~${f.kcal} kcal)` : ""}`,
    });
  }
  for (const l of p.labs) {
    events.push({
      ts_ms: l.ts_ms,
      event_type: "lab",
      summary: `Lab panel: ${l.panel} (${l.values.length} values)`,
    });
  }
  for (const i of p.imaging) {
    events.push({
      ts_ms: i.ts_ms,
      event_type: "imaging",
      summary: `${i.modality} ${i.region}`,
      detail: i.findings,
    });
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

function finalize(seed: SeedPatient): PatientDetail {
  const slug = slugify(seed.label);
  const timeline = buildTimeline(seed);
  return { ...seed, slug, timeline };
}

const PATIENTS: PatientDetail[] = [
  finalize(seedAlice()),
  finalize(seedPavel()),
  finalize(seedMarta()),
  finalize(seedJiri()),
  finalize(seedEva()),
];

// --- Public API --------------------------------------------------------------
//
// The Mock store is module-level mutable state. Submissions append in place so
// the UI can show "submitted, awaiting patient approval" without persisting.

export function listPatients(): PatientSummary[] {
  return PATIENTS.map((p) => ({
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
  return PATIENTS.find((p) => p.slug === slug);
}

/** Append a clinical note as `pending_patient_approval`. v0 mock — no real OHDC. */
export function submitNote(slug: string, text: string, author: string): ClinicalNote | null {
  const p = PATIENTS.find((x) => x.slug === slug);
  if (!p) return null;
  const note: ClinicalNote = {
    ts_ms: Date.now(),
    author,
    text,
    status: "pending_patient_approval",
  };
  p.notes.unshift(note);
  p.timeline.unshift({
    ts_ms: note.ts_ms,
    event_type: "note",
    summary: `Clinical note by ${author} [${note.status}]`,
    detail: text,
  });
  return note;
}

export function submitVital(
  slug: string,
  channel: string,
  value: number,
  unit: string,
): VitalReading | null {
  const p = PATIENTS.find((x) => x.slug === slug);
  if (!p) return null;
  const v: VitalReading = { ts_ms: Date.now(), channel, value, unit };
  p.vitals.push(v);
  p.timeline.unshift({
    ts_ms: v.ts_ms,
    event_type: "vital",
    summary: `${channel} = ${value} ${unit}`,
  });
  return v;
}

export function submitSymptom(slug: string, text: string, severity: 1 | 2 | 3 | 4 | 5): SymptomEntry | null {
  const p = PATIENTS.find((x) => x.slug === slug);
  if (!p) return null;
  const s: SymptomEntry = { ts_ms: Date.now(), text, severity };
  p.symptoms.unshift(s);
  p.timeline.unshift({
    ts_ms: s.ts_ms,
    event_type: "symptom",
    summary: `Symptom (severity ${severity}): ${text}`,
  });
  return s;
}

export function submitFood(slug: string, text: string, kcal?: number): FoodEntry | null {
  const p = PATIENTS.find((x) => x.slug === slug);
  if (!p) return null;
  const f: FoodEntry = { ts_ms: Date.now(), text, kcal };
  p.foods.unshift(f);
  p.timeline.unshift({
    ts_ms: f.ts_ms,
    event_type: "food",
    summary: `Food: ${text}${kcal ? ` (~${kcal} kcal)` : ""}`,
  });
  return f;
}

export function submitMedication(
  slug: string,
  name: string,
  dose: string,
  schedule: string,
): MedicationEntry | null {
  const p = PATIENTS.find((x) => x.slug === slug);
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
  return m;
}

export function submitLab(slug: string, panel: string, valuesText: string): LabResult | null {
  const p = PATIENTS.find((x) => x.slug === slug);
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
  return lab;
}

export function submitImaging(
  slug: string,
  modality: string,
  region: string,
  findings: string,
): ImagingStudy | null {
  const p = PATIENTS.find((x) => x.slug === slug);
  if (!p) return null;
  const i: ImagingStudy = { ts_ms: Date.now(), modality, region, findings };
  p.imaging.unshift(i);
  p.timeline.unshift({
    ts_ms: i.ts_ms,
    event_type: "imaging",
    summary: `${modality} ${region} [pending_patient_approval]`,
    detail: findings,
  });
  return i;
}

// --- Async-store API parity (no-ops in the fallback) ------------------------
//
// The OHDC-backed store needs an explicit bootstrap + subscription mechanism
// so React re-renders when the snapshot fills. The fallback is sync + always
// "ready", so these are trivial.

export function bootstrap(): Promise<void> {
  return Promise.resolve();
}

export function refresh(): Promise<void> {
  return Promise.resolve();
}

export function subscribe(_fn: () => void): () => void {
  return () => undefined;
}

export function getVersion(): number {
  return 0;
}

export function getBootstrapStatus(): { ready: boolean; error: string | null } {
  return { ready: true, error: null };
}
