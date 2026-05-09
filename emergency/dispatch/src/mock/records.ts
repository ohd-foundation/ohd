// Mock operator-records dataset (the EMS station's local DB of past cases).
//
// Per SPEC §5, the operator records DB is a Postgres schema that lives
// outside OHDC entirely; v0 ships a mock to validate the page layout.

import type { OperatorRecordRow } from "../types";

const NOW = Date.now();

export const MOCK_OPERATOR_RECORDS: OperatorRecordRow[] = [
  {
    case_ulid: "01JD9WPNNCFM3SDA90HJW8XR5T",
    patient_label: "RUN-7831",
    opened_at_ms: NOW - 3 * 3600_000,
    closed_at_ms: NOW - 2 * 3600_000,
    authority_label: "EMS Prague Region",
    destination: "Bulovka — ER",
    billing_status: "submitted",
    auto_granted: false,
  },
  {
    case_ulid: "01JD9V12K9MAFX0DQEPWBGN3T7",
    patient_label: "RUN-7826",
    opened_at_ms: NOW - 5 * 3600_000,
    closed_at_ms: NOW - 4 * 3600_000,
    authority_label: "EMS Prague Region",
    destination: "Motol — ER",
    billing_status: "submitted",
    auto_granted: true,
  },
  {
    case_ulid: "01JD9SP4XK7HW21FA3RBQDEVJM",
    patient_label: "RUN-7811",
    opened_at_ms: NOW - 24 * 3600_000,
    closed_at_ms: NOW - 23 * 3600_000,
    authority_label: "EMS Prague Region",
    destination: "VFN Praha — ER",
    billing_status: "paid",
    auto_granted: false,
  },
  {
    case_ulid: "01JD9R0X2K9NTAVMCBQGYHFD3J",
    patient_label: "RUN-7798",
    opened_at_ms: NOW - 30 * 3600_000,
    closed_at_ms: NOW - 29 * 3600_000,
    authority_label: "EMS Prague Region",
    destination: "Bulovka — ER",
    billing_status: "rejected",
    auto_granted: false,
  },
];
