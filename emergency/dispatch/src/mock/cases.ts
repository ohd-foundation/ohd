// Mock active-cases dataset for the dispatch console.
//
// Five cases spanning the three statuses (open / handoff / closed) so the
// table reflects what a busy shift looks like. Used when
// `VITE_USE_MOCK=1` (or when the OHDC bootstrap fails).

import type { CaseRow, OperatorSession, TimelineRow } from "../types";

const NOW = Date.now();

export const MOCK_OPERATOR_SESSION: OperatorSession = {
  station_label: "EMS Prague Region — Central",
  operator_display_name: "K. Novak",
  authority_cert_subject: "CN=ems-prague.cz, O=EMS Prague Region, C=CZ",
  authority_cert_expires_at_ms: NOW + 3600 * 1000 * 18, // 18h — daily Fulcio cert
  authority_cert_fingerprint: "sha256:9F:42:1A:0B:7C:DE:88:01:42:9E:F1:A8:71:4B:CD:42",
};

export const MOCK_CASES: CaseRow[] = [
  {
    case_ulid: "01JD9X8MZQK3F0VTNPB7WAYG2H",
    patient_label: "RUN-7841",
    case_type: "emergency",
    status: "open",
    opened_at_ms: NOW - 3 * 60_000,
    last_activity_ms: NOW - 12_000,
    opening_responder: "P. Horak",
    scene_note: "Praha 4, Vinohradska 1080 — chest pain, conscious",
  },
  {
    case_ulid: "01JD9X7Y1QV3FE0RA20BKDMHWS",
    patient_label: "RUN-7842",
    case_type: "emergency",
    status: "open",
    opened_at_ms: NOW - 11 * 60_000,
    last_activity_ms: NOW - 90_000,
    opening_responder: "M. Dvorak",
    scene_note: "Praha 7, Letenska 12 — fall from height, GCS 13",
  },
  {
    case_ulid: "01JD9X3K8RW1ZAPB9TGXH4NMVF",
    patient_label: "RUN-7838",
    case_type: "emergency",
    status: "handoff",
    opened_at_ms: NOW - 47 * 60_000,
    last_activity_ms: NOW - 4 * 60_000,
    opening_responder: "J. Svoboda",
    destination: "VFN Praha — ER",
    scene_note: "Praha 2, suspected stroke — handoff in progress",
  },
  {
    case_ulid: "01JD9WPNNCFM3SDA90HJW8XR5T",
    patient_label: "RUN-7831",
    case_type: "emergency",
    status: "closed",
    opened_at_ms: NOW - 3 * 3600_000,
    last_activity_ms: NOW - 2 * 3600_000,
    opening_responder: "P. Horak",
    destination: "Bulovka — ER",
    scene_note: "Closed at hospital admission",
  },
  {
    case_ulid: "01JD9V12K9MAFX0DQEPWBGN3T7",
    patient_label: "RUN-7826",
    case_type: "admission",
    status: "closed",
    opened_at_ms: NOW - 5 * 3600_000,
    last_activity_ms: NOW - 4 * 3600_000,
    opening_responder: "M. Dvorak",
    destination: "Motol — ER",
    scene_note: "Closed; routine transport",
  },
];

/** Per-case timeline rows — keyed by ULID. */
export const MOCK_TIMELINES: Record<string, TimelineRow[]> = {
  "01JD9X8MZQK3F0VTNPB7WAYG2H": [
    { ts_ms: NOW - 3 * 60_000, event_type: "case.open", summary: "Case opened by P. Horak (break-glass approved)" },
    { ts_ms: NOW - 2 * 60_000 - 30_000, event_type: "std.heart_rate_resting", summary: "HR 112 bpm" },
    { ts_ms: NOW - 2 * 60_000, event_type: "std.blood_pressure", summary: "BP 152/96" },
    { ts_ms: NOW - 90_000, event_type: "std.medication_dose", summary: "Aspirin 300 mg PO" },
    { ts_ms: NOW - 60_000, event_type: "std.observation", summary: "Pain 7/10, retrosternal" },
    { ts_ms: NOW - 12_000, event_type: "std.heart_rate_resting", summary: "HR 104 bpm" },
  ],
  "01JD9X7Y1QV3FE0RA20BKDMHWS": [
    { ts_ms: NOW - 11 * 60_000, event_type: "case.open", summary: "Case opened by M. Dvorak (break-glass approved)" },
    { ts_ms: NOW - 9 * 60_000, event_type: "std.observation", summary: "GCS 13, pupils equal", detail: "Patient responsive but disoriented." },
    { ts_ms: NOW - 6 * 60_000, event_type: "std.blood_pressure", summary: "BP 118/74" },
    { ts_ms: NOW - 90_000, event_type: "std.observation", summary: "Suspected pelvic fracture" },
  ],
  "01JD9X3K8RW1ZAPB9TGXH4NMVF": [
    { ts_ms: NOW - 47 * 60_000, event_type: "case.open", summary: "Case opened by J. Svoboda (break-glass approved)" },
    { ts_ms: NOW - 40 * 60_000, event_type: "std.observation", summary: "Suspected stroke — FAST positive" },
    { ts_ms: NOW - 30 * 60_000, event_type: "std.medication_dose", summary: "O2 4L/min via mask" },
    { ts_ms: NOW - 10 * 60_000, event_type: "case.handoff_initiated", summary: "Handoff to VFN Praha ER initiated" },
    { ts_ms: NOW - 4 * 60_000, event_type: "std.clinical_note", summary: "Handoff summary recorded", detail: "Patient transferred to ER team Dr. Beran." },
  ],
  "01JD9WPNNCFM3SDA90HJW8XR5T": [
    { ts_ms: NOW - 3 * 3600_000, event_type: "case.open", summary: "Case opened by P. Horak" },
    { ts_ms: NOW - 2 * 3600_000 - 50 * 60_000, event_type: "std.observation", summary: "Stable vitals" },
    { ts_ms: NOW - 2 * 3600_000, event_type: "case.handoff_complete", summary: "Handed off to Bulovka ER" },
    { ts_ms: NOW - 2 * 3600_000 + 60_000, event_type: "case.close", summary: "Case closed" },
  ],
  "01JD9V12K9MAFX0DQEPWBGN3T7": [
    { ts_ms: NOW - 5 * 3600_000, event_type: "case.open", summary: "Case opened by M. Dvorak" },
    { ts_ms: NOW - 4 * 3600_000 - 30 * 60_000, event_type: "case.handoff_complete", summary: "Handed off to Motol ER" },
    { ts_ms: NOW - 4 * 3600_000, event_type: "case.close", summary: "Case closed" },
  ],
};
