// Operator-side audit log per `care/SPEC.md` §7.2 (`care_operator_audit`).
//
// One row per OHDC RPC. The `query_hash` is computed locally before the call
// goes out (see `canonicalQueryHash.ts`); storage records the same hash on
// the patient side. Joining the two by `(grant_id, query_hash, ts_ms)`
// recovers the cross-side audit trail (§7.3).
//
// Web persistence: localStorage. v0 keeps a rolling 1000-entry buffer per
// browser session; deployment-grade persistence layers a Postgres / SQLite
// backend behind the same shape (the next pass per `care/STATUS.md`).

import type { CanonicalEventFilter, CanonicalQueryKind } from "./canonicalQueryHash";

const STORAGE_KEY = "ohd-care-operator-audit";
const MAX_ENTRIES = 1000;

/** One operator-side audit row. Shape mirrors the eventual SQL schema. */
export interface OperatorAuditEntry {
  /** Unix ms when the call was *issued* (pre-RPC). */
  tsMs: number;
  /**
   * OIDC `sub` of the operator who fired the call (when available); `null`
   * for legacy `--operator-token` sessions where we don't yet have the
   * subject. Mirrors `oidc-login`'s vault entry.
   */
  operatorSubject: string | null;
  /**
   * The grant ULID being used. Care holds the grant's wire ULID locally
   * (the rendezvous URL or a `?token=...` ingest). Empty string for
   * pre-multi-grant v0 (single-grant browser session).
   */
  grantUlid: string;
  /** OHDC RPC name, e.g. `query_events`, `put_events`. */
  ohdcAction: string;
  /** Canonical query hash (hex). Joins to the patient-side audit row. */
  queryHash: string | null;
  /**
   * One of the `query_kind` strings storage recognises. Set for read RPCs;
   * `null` for write / lifecycle RPCs that don't go through the
   * pending-query path.
   */
  queryKind: CanonicalQueryKind | null;
  /** Outcome — narrowed mirror of storage's `audit_log.result`. */
  result: "success" | "partial" | "rejected" | "error" | "pending";
  /** Rows returned (read RPCs); null for writes. */
  rowsReturned: number | null;
  /** Rows silently filtered (read RPCs); null for writes. */
  rowsFiltered: number | null;
  /** Optional reason / error code; surfaced on `rejected` / `error`. */
  reason: string | null;
}

/** Append one row. Trims to MAX_ENTRIES, keeping the most-recent. */
export function appendOperatorAuditEntry(entry: OperatorAuditEntry): void {
  if (typeof window === "undefined") return;
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    const buf: OperatorAuditEntry[] = raw ? JSON.parse(raw) : [];
    buf.push(entry);
    if (buf.length > MAX_ENTRIES) {
      buf.splice(0, buf.length - MAX_ENTRIES);
    }
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(buf));
  } catch {
    // localStorage may be unavailable (private mode, quota) — silently drop;
    // the operator-side audit is best-effort on the web client.
  }
}

/** Read the buffered rows. Newest last (append order). */
export function readOperatorAudit(): OperatorAuditEntry[] {
  if (typeof window === "undefined") return [];
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as OperatorAuditEntry[]) : [];
  } catch {
    return [];
  }
}

/** Wipe the buffer. Test hook + "logout" path. */
export function clearOperatorAudit(): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(STORAGE_KEY);
  } catch {
    // ignore
  }
}

/**
 * Pre-baked record builder for read RPCs. The caller computes the hash via
 * `canonicalQueryHash` before issuing the call, then either passes the
 * resolved entry on success or augments with `result/rows/reason` once the
 * RPC returns.
 *
 * Call sites are in `client.ts` (`queryEvents`, etc.).
 */
export function buildAuditTemplate(
  ohdcAction: string,
  queryKind: CanonicalQueryKind | null,
  filter: CanonicalEventFilter | null,
  queryHashHex: string | null,
): OperatorAuditEntry {
  void filter; // captured implicitly in the hash; not stored verbatim
  return {
    tsMs: Date.now(),
    operatorSubject: readOperatorSubject(),
    grantUlid: readActiveGrantUlid(),
    ohdcAction,
    queryHash: queryHashHex,
    queryKind,
    result: "success",
    rowsReturned: null,
    rowsFiltered: null,
    reason: null,
  };
}

function readOperatorSubject(): string | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.sessionStorage.getItem("ohd-care-operator-session");
    if (!raw) return null;
    const session = JSON.parse(raw) as { oidcSubject?: string };
    return session.oidcSubject ?? null;
  } catch {
    return null;
  }
}

function readActiveGrantUlid(): string {
  if (typeof window === "undefined") return "";
  try {
    return window.sessionStorage.getItem("ohd-care-grant-ulid") ?? "";
  } catch {
    return "";
  }
}
