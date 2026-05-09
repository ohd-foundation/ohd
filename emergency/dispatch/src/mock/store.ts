// Backend selector: mock vs. OHDC.
//
// Default = OHDC (real Connect-RPC client at `../ohdc/store`).
// `VITE_USE_MOCK=1` flips to the mock backend below — no network required,
// great for visual review and Vitest smoke tests.
//
// Both backends export the same surface so call sites don't have to branch.

import type { CaseRow, CrewMember, AuditRow, OperatorRecordRow, OperatorSession, TimelineRow } from "../types";
import * as ohdcStore from "../ohdc/store";
import { MOCK_CASES, MOCK_OPERATOR_SESSION, MOCK_TIMELINES } from "./cases";
import { MOCK_CREW } from "./crew";
import { MOCK_AUDIT } from "./audit";
import { MOCK_OPERATOR_RECORDS } from "./records";

const USE_MOCK = (import.meta.env?.VITE_USE_MOCK as string | undefined) === "1";

// --- Mock backend ----------------------------------------------------------

const mockSubscribers = new Set<() => void>();
let mockVersion = 0;
let mockReady = false;
let mockCases: CaseRow[] = [...MOCK_CASES];

function mockNotify() {
  mockVersion += 1;
  for (const s of mockSubscribers) s();
}

const mockBackend = {
  bootstrap: async () => {
    mockReady = true;
    mockNotify();
  },
  refresh: async () => {
    // Touch last_activity_ms on the open cases so the table looks live.
    const now = Date.now();
    mockCases = mockCases.map((c) =>
      c.status === "open"
        ? { ...c, last_activity_ms: Math.max(c.last_activity_ms, now - 5_000) }
        : c,
    );
    mockNotify();
  },
  subscribe: (fn: () => void): (() => void) => {
    mockSubscribers.add(fn);
    return () => {
      mockSubscribers.delete(fn);
    };
  },
  getVersion: (): number => mockVersion,
  getBootstrapStatus: (): { ready: boolean; error: string | null } => ({
    ready: mockReady,
    error: null,
  }),
  getSession: (): OperatorSession => MOCK_OPERATOR_SESSION,
  getCases: (): CaseRow[] => mockCases,
  fetchCaseRow: async (ulid: string): Promise<CaseRow | null> =>
    mockCases.find((c) => c.case_ulid === ulid) ?? null,
  forceCloseCase: async (ulid: string, _reason: string): Promise<boolean> => {
    const idx = mockCases.findIndex((c) => c.case_ulid === ulid);
    if (idx < 0) return false;
    mockCases = mockCases.map((c, i) =>
      i === idx ? { ...c, status: "closed" as const } : c,
    );
    mockNotify();
    return true;
  },
};

// --- Selected backend ------------------------------------------------------

const backend = USE_MOCK ? mockBackend : ohdcStore;

export const isMockMode = USE_MOCK;

export const bootstrap = backend.bootstrap;
export const refresh = backend.refresh;
export const subscribe = backend.subscribe;
export const getVersion = backend.getVersion;
export const getBootstrapStatus = backend.getBootstrapStatus;
export const getSession = backend.getSession;
export const getCases = backend.getCases;
export const fetchCaseRow = backend.fetchCaseRow;
export const forceCloseCase = backend.forceCloseCase;

// --- Mock-only readers (no OHDC equivalent yet) ---------------------------

/**
 * Crew, audit, records, and per-case timelines have no OHDC-side wiring
 * yet — they're operator-side data (crew, records) or stubbed RPCs
 * (AuditQuery). Until those land we return mock data unconditionally so
 * the pages remain reviewable.
 */
export function getCrew(): CrewMember[] {
  return MOCK_CREW;
}

export function getAudit(): AuditRow[] {
  return MOCK_AUDIT;
}

export function getOperatorRecords(): OperatorRecordRow[] {
  return MOCK_OPERATOR_RECORDS;
}

export function getTimeline(caseUlid: string): TimelineRow[] {
  return MOCK_TIMELINES[caseUlid] ?? [];
}
