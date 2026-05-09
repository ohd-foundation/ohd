// Domain types used by the dispatch console.
//
// These mirror what the OHDC `Case` proto carries plus a few operator-side
// fields (paramedic_label, opening_responder) that the dispatch DB tracks
// outside OHDC. The OHDC store hydrates the OHDC-owned fields from
// `OhdcService.ListCases` / `GetCase`; the operator-side fields stay mock
// for v0 until the operator records DB lands (see SPEC §5).

/** Status of an OHDC case as the dispatcher sees it. */
export type CaseStatus = "open" | "handoff" | "closed";

/** A row on the active-cases table. */
export interface CaseRow {
  /** 26-char Crockford-base32 ULID. */
  case_ulid: string;
  /**
   * Operator-local label for the patient. The OHDC side never reveals patient
   * identity to the operator beyond the case grant; "patient_label" is the
   * dispatcher's own shorthand (often the run-sheet number).
   */
  patient_label: string;
  case_type: string;
  status: CaseStatus;
  opened_at_ms: number;
  /** UTC ms of the last event recorded against this case. */
  last_activity_ms: number;
  /**
   * Operator-side: which paramedic (by operator-IdP label) opened the case.
   * Comes from the operator records DB, not OHDC.
   */
  opening_responder: string;
  /** Receiving facility on handoff (if any). Operator-side. */
  destination?: string;
  /** Free-text scene note for the dispatcher. Operator-side. */
  scene_note?: string;
}

/** A row in the case detail timeline drawer. */
export interface TimelineRow {
  ts_ms: number;
  event_type: string;
  /** Short, one-line summary suitable for a dense table. */
  summary: string;
  /** Optional longer text (notes, handoff text). */
  detail?: string;
}

/** A paramedic on the operator's roster. */
export interface CrewMember {
  responder_label: string;
  /** Display name (operator IdP). */
  display_name: string;
  on_duty: boolean;
  on_duty_since_ms: number | null;
  /** ULID of the case the paramedic is currently assigned to, if any. */
  current_case_ulid: string | null;
  last_seen_ms: number;
  contact: string;
}

/** Operator break-glass audit row. */
export interface AuditRow {
  ts_ms: number;
  responder_label: string;
  case_ulid: string;
  action: string;
  /** 'success' | 'partial' | 'rejected' | 'error' (mirrors AuditEntry.result) */
  result: string;
  /** What was accessed/written, summarized. */
  scope: string;
  caller_ip?: string;
}

/** A row in the operator-records page (post-close case archive). */
export interface OperatorRecordRow {
  case_ulid: string;
  patient_label: string;
  opened_at_ms: number;
  closed_at_ms: number;
  authority_label: string;
  destination: string;
  billing_status: string;
  auto_granted: boolean;
}

/** Active operator session. Driven by the bearer token + WhoAmI. */
export interface OperatorSession {
  station_label: string;
  operator_display_name: string;
  authority_cert_subject: string;
  authority_cert_expires_at_ms: number;
  authority_cert_fingerprint: string;
}
