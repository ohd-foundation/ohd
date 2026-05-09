// Shared types for the OHD Care v0 web shell.
//
// These mirror the shapes Care will eventually receive from OHDC (events,
// medications, labs, notes, etc.). For v0 they're populated from the in-memory
// mock store at src/mock/store.ts — no backend, no localStorage.

export type EventType =
  | "vital"
  | "medication"
  | "symptom"
  | "food"
  | "lab"
  | "imaging"
  | "note";

export type ApprovalMode =
  | "always"
  | "auto_for_event_types"
  | "never_required";

export type GrantStatus = "active" | "expiring_soon" | "case_bound";

/** A roster row — one patient who has granted the operator access. */
export interface PatientSummary {
  /** Operator-typed local identifier ("Alice (DOB 1985-04-12)"). Never sent to storage. */
  label: string;
  /** URL slug derived from the label. Used in /patient/:label routes. */
  slug: string;
  /** Display name as the patient chose to share via Auth.WhoAmI. */
  display_name: string;
  /** Last visit timestamp in ms since epoch. null = never visited. */
  last_visit_ms: number | null;
  /** Short flag chips shown on the roster row ("BP trending up", "missed 4/14 doses"). */
  flags: string[];
  /** Up to 3 lines of current-meds summary for the roster card. */
  meds_summary: string[];
  /** Active grant info — drives the per-patient header. */
  grant: {
    read_scope: string[];
    write_scope: string[];
    approval_mode: ApprovalMode;
    /** Expiry timestamp in ms since epoch, or null if open-ended. */
    expires_at_ms: number | null;
    status: GrantStatus;
    /** Set when the grant is case-bound (e.g. patient-curated visit). */
    case_label?: string;
    case_event_count?: number;
  };
  /** Active case at the patient's storage (e.g. EMS handoff in progress). */
  active_case?: {
    label: string;
    authority: string;
    started_ms: number;
  };
}

export interface VitalReading {
  ts_ms: number;
  channel: string; // 'bp_systolic' | 'bp_diastolic' | 'hr' | 'temp_c' | 'spo2' | 'glucose_mg_dl'
  value: number;
  unit: string;
}

export interface MedicationEntry {
  name: string;
  dose: string;
  schedule: string;
  /** Recent dose log: timestamps + taken/missed flag. */
  recent_doses: { ts_ms: number; taken: boolean }[];
  active: boolean;
}

export interface SymptomEntry {
  ts_ms: number;
  text: string;
  severity: 1 | 2 | 3 | 4 | 5;
}

export interface FoodEntry {
  ts_ms: number;
  text: string;
  kcal?: number;
}

export interface LabResult {
  ts_ms: number;
  panel: string;
  values: { name: string; value: string; range?: string; flag?: "low" | "high" | "normal" }[];
}

export interface ImagingStudy {
  ts_ms: number;
  modality: string; // 'X-ray' | 'CT' | 'MRI' | 'US'
  region: string;
  findings: string;
}

export interface ClinicalNote {
  ts_ms: number;
  author: string;
  text: string;
  /** Submission status for write-with-approval. */
  status: "committed" | "pending_patient_approval" | "auto_committed";
}

export interface TimelineEvent {
  ts_ms: number;
  event_type: EventType;
  summary: string;
  detail?: string;
}

export interface PatientDetail extends PatientSummary {
  brief: string[]; // visit-prep bullets
  vitals: VitalReading[];
  medications: MedicationEntry[];
  symptoms: SymptomEntry[];
  foods: FoodEntry[];
  labs: LabResult[];
  imaging: ImagingStudy[];
  notes: ClinicalNote[];
  timeline: TimelineEvent[];
}

export interface Operator {
  display_name: string;
  role: "clinician" | "nurse" | "admin" | "auditor";
  status: "online" | "idle";
}
