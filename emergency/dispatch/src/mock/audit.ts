// Mock audit dataset.
//
// Storage's `OhdcService.AuditQuery` is stubbed today (returns NOT_IMPLEMENTED;
// see ../../../storage/STATUS.md). The dispatch UI shows this dataset under
// a "TBD: storage AuditQuery RPC pending" banner so layout/density is
// reviewable now and the swap to the real RPC is a single store change.

import type { AuditRow } from "../types";

const NOW = Date.now();

export const MOCK_AUDIT: AuditRow[] = [
  {
    ts_ms: NOW - 30_000,
    responder_label: "horak.p",
    case_ulid: "01JD9X8MZQK3F0VTNPB7WAYG2H",
    action: "read",
    result: "success",
    scope: "std.allergies, std.active_medications",
    caller_ip: "10.4.18.42",
  },
  {
    ts_ms: NOW - 90_000,
    responder_label: "horak.p",
    case_ulid: "01JD9X8MZQK3F0VTNPB7WAYG2H",
    action: "write",
    result: "success",
    scope: "std.medication_dose (Aspirin)",
    caller_ip: "10.4.18.42",
  },
  {
    ts_ms: NOW - 3 * 60_000,
    responder_label: "horak.p",
    case_ulid: "01JD9X8MZQK3F0VTNPB7WAYG2H",
    action: "break_glass.initiate",
    result: "success",
    scope: "patient grant approved (1.4s)",
    caller_ip: "10.4.18.42",
  },
  {
    ts_ms: NOW - 4 * 60_000,
    responder_label: "svoboda.j",
    case_ulid: "01JD9X3K8RW1ZAPB9TGXH4NMVF",
    action: "write",
    result: "success",
    scope: "std.clinical_note (handoff summary)",
    caller_ip: "10.4.18.51",
  },
  {
    ts_ms: NOW - 11 * 60_000,
    responder_label: "dvorak.m",
    case_ulid: "01JD9X7Y1QV3FE0RA20BKDMHWS",
    action: "break_glass.initiate",
    result: "success",
    scope: "patient grant approved (auto-grant: timeout)",
    caller_ip: "10.4.18.43",
  },
  {
    ts_ms: NOW - 22 * 60_000,
    responder_label: "kralova.i",
    case_ulid: "01JD9X1L9PMV7HJZE8N4QKW3RT",
    action: "break_glass.initiate",
    result: "rejected",
    scope: "patient declined break-glass",
    caller_ip: "10.4.18.44",
  },
  {
    ts_ms: NOW - 47 * 60_000,
    responder_label: "svoboda.j",
    case_ulid: "01JD9X3K8RW1ZAPB9TGXH4NMVF",
    action: "break_glass.initiate",
    result: "success",
    scope: "patient grant approved",
    caller_ip: "10.4.18.51",
  },
  {
    ts_ms: NOW - 2 * 3600_000,
    responder_label: "horak.p",
    case_ulid: "01JD9WPNNCFM3SDA90HJW8XR5T",
    action: "case.close",
    result: "success",
    scope: "case closed at handoff",
    caller_ip: "10.4.18.42",
  },
  {
    ts_ms: NOW - 3 * 3600_000,
    responder_label: "horak.p",
    case_ulid: "01JD9WPNNCFM3SDA90HJW8XR5T",
    action: "break_glass.initiate",
    result: "success",
    scope: "patient grant approved",
    caller_ip: "10.4.18.42",
  },
  {
    ts_ms: NOW - 4 * 3600_000,
    responder_label: "dvorak.m",
    case_ulid: "01JD9V12K9MAFX0DQEPWBGN3T7",
    action: "break_glass.initiate",
    result: "partial",
    scope: "patient grant approved with restricted scope (no labs)",
    caller_ip: "10.4.18.43",
  },
];
