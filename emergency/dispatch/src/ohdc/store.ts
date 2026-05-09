// OHDC-backed store for the dispatch console.
//
// Surface mirrors `../mock/store.ts` so pages can be backend-agnostic. The
// snapshot is hydrated by `bootstrap()` (WhoAmI + initial ListCases) and
// refreshed on a 5s polling timer while the page is active. Real-time
// websocket / SSE updates land in v0.x.

import { ulidToCrockford } from "../util";
import {
  closeCase as ohdcCloseCase,
  getCase,
  listCases,
  resolveOperatorToken,
  whoAmI,
  type Case,
} from "./client";
import type { CaseRow, OperatorSession } from "../types";
import { MOCK_OPERATOR_SESSION } from "../mock/cases";

interface Snapshot {
  ready: boolean;
  error: string | null;
  session: OperatorSession;
  cases: CaseRow[];
}

let snapshot: Snapshot = {
  ready: false,
  error: null,
  session: MOCK_OPERATOR_SESSION,
  cases: [],
};

const subscribers = new Set<() => void>();
let version = 0;

function notify() {
  version += 1;
  for (const s of subscribers) s();
}

export function subscribe(fn: () => void): () => void {
  subscribers.add(fn);
  return () => {
    subscribers.delete(fn);
  };
}

export function getVersion(): number {
  return version;
}

export function getBootstrapStatus(): { ready: boolean; error: string | null } {
  return { ready: snapshot.ready, error: snapshot.error };
}

export function getSession(): OperatorSession {
  return snapshot.session;
}

export function getCases(): CaseRow[] {
  return snapshot.cases;
}

let bootstrapping: Promise<void> | null = null;

export function bootstrap(): Promise<void> {
  if (bootstrapping) return bootstrapping;
  bootstrapping = (async () => {
    const token = resolveOperatorToken();
    if (!token) {
      snapshot = { ...snapshot, ready: true, error: "no_token" };
      notify();
      return;
    }
    try {
      const me = await whoAmI();
      if (!me) {
        snapshot = { ...snapshot, ready: true, error: "whoami_failed" };
        notify();
        return;
      }
      // Preserve mock authority-cert info — WhoAmI doesn't carry it; that's
      // an operator-side artifact (the relay's authority cert chain). v0.x
      // adds a /healthz/cert lookup against the relay.
      const session: OperatorSession = {
        ...MOCK_OPERATOR_SESSION,
        operator_display_name: me.granteeLabel || MOCK_OPERATOR_SESSION.operator_display_name,
      };
      const cases = await listCases(true);
      snapshot = {
        ready: true,
        error: null,
        session,
        cases: cases.map(caseToRow),
      };
      notify();
    } catch (err) {
      // eslint-disable-next-line no-console
      console.error("OHDC dispatch bootstrap failed", err);
      snapshot = {
        ...snapshot,
        ready: true,
        error: `bootstrap_failed: ${(err as Error).message ?? String(err)}`,
      };
      notify();
    }
  })();
  return bootstrapping;
}

export async function refresh(): Promise<void> {
  const token = resolveOperatorToken();
  if (!token) return;
  try {
    const cases = await listCases(true);
    snapshot = { ...snapshot, cases: cases.map(caseToRow) };
    notify();
  } catch (err) {
    // eslint-disable-next-line no-console
    console.warn("OHDC dispatch refresh failed", err);
  }
}

/**
 * Fetch a single case by its 26-char Crockford ULID. Used by the case
 * detail drawer to refresh just the one row + its metadata.
 */
export async function fetchCaseRow(ulid: string): Promise<CaseRow | null> {
  const bytes = crockfordToUlidBytes(ulid);
  if (!bytes) return null;
  const c = await getCase(bytes);
  if (!c) return null;
  return caseToRow(c);
}

export async function forceCloseCase(ulid: string, reason: string): Promise<boolean> {
  const bytes = crockfordToUlidBytes(ulid);
  if (!bytes) return false;
  const updated = await ohdcCloseCase(bytes, reason);
  if (!updated) return false;
  await refresh();
  return true;
}

// --- helpers ---------------------------------------------------------------

function caseToRow(c: Case): CaseRow {
  const ulid = c.ulid?.bytes ? ulidToCrockford(c.ulid.bytes) : "";
  const opened = Number(c.startedAtMs ?? 0n);
  const last = Number(c.lastActivityAtMs ?? c.startedAtMs ?? 0n);
  const ended = c.endedAtMs ? Number(c.endedAtMs) : null;
  const status: CaseRow["status"] = ended != null ? "closed" : "open";
  return {
    case_ulid: ulid,
    patient_label: c.caseLabel || "(unlabeled)",
    case_type: c.caseType || "emergency",
    status,
    opened_at_ms: opened,
    last_activity_ms: last,
    // Operator-side fields aren't on the OHDC `Case`; fall back to "—" until
    // the operator records DB ships and these get joined client-side.
    opening_responder: "—",
  };
}

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const CROCKFORD_INDEX: Record<string, number> = (() => {
  const m: Record<string, number> = {};
  for (let i = 0; i < CROCKFORD.length; i++) m[CROCKFORD[i]] = i;
  // Crockford alias: I → 1, L → 1, O → 0
  m["I"] = 1;
  m["L"] = 1;
  m["O"] = 0;
  return m;
})();

function crockfordToUlidBytes(s: string): Uint8Array | null {
  const up = s.toUpperCase();
  if (up.length !== 26) return null;
  const bytes = new Uint8Array(16);
  let bitBuf = 0;
  let bitCount = 0;
  let outIdx = 0;
  for (let i = 0; i < up.length; i++) {
    const v = CROCKFORD_INDEX[up[i]];
    if (v == null) return null;
    if (i === 0) {
      // First char encodes only top 3 bits.
      bitBuf = v & 0x07;
      bitCount = 3;
    } else {
      bitBuf = (bitBuf << 5) | (v & 0x1f);
      bitCount += 5;
    }
    while (bitCount >= 8 && outIdx < 16) {
      bitCount -= 8;
      bytes[outIdx++] = (bitBuf >> bitCount) & 0xff;
    }
  }
  return outIdx === 16 ? bytes : null;
}
