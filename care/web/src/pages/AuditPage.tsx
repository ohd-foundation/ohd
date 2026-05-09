// AuditPage.tsx — two-sided audit panel.
//
// Per `care/SPEC.md` §7 we surface BOTH the patient-side audit log
// (queried via `OhdcService.AuditQuery`) and the operator-side audit
// log (`care/web/src/ohdc/operatorAudit.ts`, populated on every OHDC
// dispatch with the canonical `query_hash`).
//
// The JOIN is by `query_hash`: storage emits `(query_kind,
// query_params_json)` per row, we re-hash on the client side
// (`canonicalQueryHashFromRawJson`) and look up our local row by the
// same hash. Rows without a counterpart on either side are surfaced
// with an asymmetry badge — that's the real audit signal:
//   - storage row, no operator row → storage saw something we don't
//     think we sent (token leak? audit-log tampering on our side?).
//   - operator row, no storage row → we issued a call that storage
//     didn't record (eg. it didn't reach storage; or storage's audit
//     write failed silently).
//
// Filter chips (top of page):
//   - Actor: self / grant (each grant ULID we hold is a chip; v0 is
//     one-grant so we collapse to "self" + "this grant").
//   - Op kind: read / write / pending / case-op (mapped from
//     `action`). Free-text filter lives behind an "advanced" disclosure.
//   - Time range: last 1h / 24h / 7d / 30d.
//
// Export → CSV (joined view, one row per JOIN bucket; ULIDs as
// Crockford strings; hashes truncated to 16 chars in the UI but full
// 64 in the CSV).

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  auditQuery,
  ulidToCrockford,
  type AuditEntry,
  type AuditQueryFilter,
} from "../ohdc/client";
import { canonicalQueryHashFromRawJson } from "../ohdc/canonicalQueryHash";
import {
  readOperatorAudit,
  type OperatorAuditEntry,
} from "../ohdc/operatorAudit";
import { useToast } from "../components/Toast";

// --- Time-range chips -------------------------------------------------------

type TimeWindow = "1h" | "24h" | "7d" | "30d" | "all";

const WINDOWS: { id: TimeWindow; label: string; ms: number | null }[] = [
  { id: "1h", label: "Last 1h", ms: 60 * 60 * 1000 },
  { id: "24h", label: "Last 24h", ms: 24 * 60 * 60 * 1000 },
  { id: "7d", label: "Last 7 days", ms: 7 * 24 * 60 * 60 * 1000 },
  { id: "30d", label: "Last 30 days", ms: 30 * 24 * 60 * 60 * 1000 },
  { id: "all", label: "All time", ms: null },
];

type ActorChip = "all" | "self" | "grant";
type OpChip = "all" | "read" | "write" | "pending" | "case_op";

const OP_BUCKETS: Record<OpChip, string[]> = {
  all: [],
  read: ["read", "query_events", "aggregate", "correlate", "read_samples", "read_attachment", "get_event_by_ulid"],
  write: ["write", "put_events"],
  pending: ["list_pending", "approve_pending", "reject_pending"],
  case_op: ["open_case", "close_case", "list_cases", "case_op", "force_close_case"],
};

// --- JOIN model -------------------------------------------------------------

/**
 * One row in the joined view. The two sides are independent — either
 * may be null. The `queryHash` is what we JOIN on (when populated on
 * both sides); for write / pending / case-op rows the `query_kind`
 * isn't a pending-query kind so storage emits the action string and
 * we fall back to `(grant, ts ± 5s, action)` matching.
 */
interface JoinedRow {
  /** Stable id for keyed rendering — `${storageIdx}-${operatorIdx}`. */
  id: string;
  storage: AuditEntry | null;
  operator: OperatorAuditEntry | null;
  /** Hash used for the JOIN, when computable. Empty for non-pending-query rows. */
  joinHash: string;
  /** True when only one side has data — surface as a red asymmetry badge. */
  asymmetric: boolean;
  /** The asymmetry side, when applicable. */
  asymmetrySide: "storage_only" | "operator_only" | null;
  /** Effective ts for sort; ms-since-epoch. */
  tsMs: number;
}

const SYMMETRY_WINDOW_MS = 5_000;

/** Compute the JOIN hash for a storage row. Returns "" when not joinable. */
async function storageJoinHash(entry: AuditEntry): Promise<string> {
  // Only the OHDC pending-query `query_kind`s join cleanly. For every
  // other action we fall back to bucket matching by (action, ts).
  const KIND_BUCKET = new Set([
    "query_events",
    "aggregate",
    "correlate",
    "read_samples",
    "read_attachment",
    "get_event_by_ulid",
  ]);
  if (!KIND_BUCKET.has(entry.queryKind)) return "";
  if (!entry.queryParamsJson) return "";
  return canonicalQueryHashFromRawJson(entry.queryKind, entry.queryParamsJson);
}

/**
 * Build the joined view.
 *
 *  - For each storage row, try to find an operator-side row with the same
 *    `queryHash` (when both sides have one), or `(action, ts ± 5s)`
 *    fallback.
 *  - Then, every still-unmatched operator-side row is appended with
 *    `storage = null` (operator-only — we recorded the call, storage
 *    didn't audit it).
 */
async function buildJoinedRows(
  storageRows: AuditEntry[],
  operatorRows: OperatorAuditEntry[],
): Promise<JoinedRow[]> {
  const out: JoinedRow[] = [];
  // Index operator rows by queryHash (if any), and per-action time-buckets.
  const byHash = new Map<string, OperatorAuditEntry[]>();
  const byAction: OperatorAuditEntry[] = [];
  for (const op of operatorRows) {
    if (op.queryHash) {
      const list = byHash.get(op.queryHash) ?? [];
      list.push(op);
      byHash.set(op.queryHash, list);
    }
    byAction.push(op);
  }
  // Track which operator rows have been consumed.
  const consumed = new Set<OperatorAuditEntry>();

  for (let si = 0; si < storageRows.length; si++) {
    const sRow = storageRows[si];
    const tsMs = Number(sRow.tsMs);
    const joinHash = await storageJoinHash(sRow);
    let match: OperatorAuditEntry | null = null;
    if (joinHash) {
      const candidates = byHash.get(joinHash) ?? [];
      for (const c of candidates) {
        if (consumed.has(c)) continue;
        // Tolerate small clock drift between client and server.
        if (Math.abs(c.tsMs - tsMs) <= 60_000) {
          match = c;
          break;
        }
      }
    }
    if (!match) {
      // Fallback: action + close-in-time. Map storage action to our
      // ohdcAction strings (`query_events`, `put_events`, …).
      for (const c of byAction) {
        if (consumed.has(c)) continue;
        if (!actionsMatch(sRow.action, sRow.queryKind, c.ohdcAction)) continue;
        if (Math.abs(c.tsMs - tsMs) > SYMMETRY_WINDOW_MS) continue;
        match = c;
        break;
      }
    }
    if (match) consumed.add(match);
    out.push({
      id: `s${si}-${match ? `o${operatorRows.indexOf(match)}` : "x"}`,
      storage: sRow,
      operator: match,
      joinHash,
      asymmetric: !match,
      asymmetrySide: match ? null : "storage_only",
      tsMs,
    });
  }

  // Operator-only rows (storage didn't audit).
  for (let oi = 0; oi < operatorRows.length; oi++) {
    const op = operatorRows[oi];
    if (consumed.has(op)) continue;
    out.push({
      id: `x-o${oi}`,
      storage: null,
      operator: op,
      joinHash: op.queryHash ?? "",
      asymmetric: true,
      asymmetrySide: "operator_only",
      tsMs: op.tsMs,
    });
  }

  // Newest first.
  out.sort((a, b) => b.tsMs - a.tsMs);
  return out;
}

function actionsMatch(
  storageAction: string,
  storageKind: string,
  operatorAction: string,
): boolean {
  if (storageAction === operatorAction) return true;
  if (storageKind === operatorAction) return true;
  // Loose mappings — storage uses higher-level action names; operator
  // logs the OHDC RPC name.
  const RPC_TO_ACTION: Record<string, string> = {
    query_events: "read",
    put_events: "write",
    list_pending: "list_pending",
    approve_pending: "approve_pending",
    reject_pending: "reject_pending",
    audit_query: "audit_query",
  };
  return RPC_TO_ACTION[operatorAction] === storageAction;
}

// --- Component --------------------------------------------------------------

export function AuditPage() {
  const toast = useToast();
  const [loading, setLoading] = useState<boolean>(true);
  const [error, setError] = useState<string | null>(null);
  const [actorChip, setActorChip] = useState<ActorChip>("all");
  const [opChip, setOpChip] = useState<OpChip>("all");
  const [windowChip, setWindowChip] = useState<TimeWindow>("24h");
  const [joined, setJoined] = useState<JoinedRow[]>([]);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const win = WINDOWS.find((w) => w.id === windowChip)!;
      const filter: AuditQueryFilter = {
        fromMs: win.ms == null ? undefined : Date.now() - win.ms,
        actorType: actorChip === "all" ? undefined : actorChip,
      };
      const rows = await auditQuery(filter);
      const operator = readOperatorAudit();
      // Apply window + actor + op-bucket filters on the operator side too,
      // since storage already does it for us on the storage side.
      const fromMs = filter.fromMs ?? 0;
      const filteredOperator: OperatorAuditEntry[] = operator.filter((o) => {
        if (o.tsMs < fromMs) return false;
        if (actorChip === "self") {
          // operator-side "self" doesn't have a clean concept; treat as
          // any row not bound to a grant ulid.
          if (o.grantUlid && o.grantUlid !== "") return false;
        } else if (actorChip === "grant") {
          if (!o.grantUlid) return false;
        }
        if (opChip !== "all") {
          const bucket = OP_BUCKETS[opChip];
          if (!bucket.includes(o.ohdcAction)) return false;
        }
        return true;
      });
      // Same op-bucket filter on storage rows (storage's `action` field
      // is an enum string; pending-query kinds are in `queryKind`).
      const filteredStorage: AuditEntry[] = rows.filter((r) => {
        if (opChip === "all") return true;
        const bucket = OP_BUCKETS[opChip];
        return bucket.includes(r.action) || bucket.includes(r.queryKind);
      });
      const j = await buildJoinedRows(filteredStorage, filteredOperator);
      setJoined(j);
    } catch (err) {
      setError((err as Error).message ?? String(err));
    } finally {
      setLoading(false);
    }
  }, [actorChip, opChip, windowChip]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const exportCsv = useCallback(() => {
    const lines: string[] = [];
    lines.push(
      [
        "ts_iso",
        "asymmetry",
        "join_hash",
        "storage_actor_type",
        "storage_grant_ulid",
        "storage_action",
        "storage_query_kind",
        "storage_rows_returned",
        "storage_rows_filtered",
        "storage_result",
        "operator_subject",
        "operator_grant_ulid",
        "operator_action",
        "operator_query_kind",
        "operator_rows_returned",
        "operator_result",
      ].join(","),
    );
    for (const r of joined) {
      const iso = new Date(r.tsMs).toISOString();
      const sg = r.storage?.grantUlid ? ulidToCrockford(r.storage.grantUlid.bytes) : "";
      lines.push(
        [
          iso,
          r.asymmetrySide ?? "matched",
          r.joinHash,
          r.storage?.actorType ?? "",
          sg,
          r.storage?.action ?? "",
          r.storage?.queryKind ?? "",
          r.storage?.rowsReturned != null ? String(r.storage.rowsReturned) : "",
          r.storage?.rowsFiltered != null ? String(r.storage.rowsFiltered) : "",
          r.storage?.result ?? "",
          r.operator?.operatorSubject ?? "",
          r.operator?.grantUlid ?? "",
          r.operator?.ohdcAction ?? "",
          r.operator?.queryKind ?? "",
          r.operator?.rowsReturned != null ? String(r.operator.rowsReturned) : "",
          r.operator?.result ?? "",
        ]
          .map(csvEscape)
          .join(","),
      );
    }
    const blob = new Blob([lines.join("\n")], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `audit-joined-${new Date().toISOString().slice(0, 10)}.csv`;
    a.click();
    URL.revokeObjectURL(url);
    toast.show(`Exported ${joined.length} audit rows`, "success");
  }, [joined, toast]);

  const counts = useMemo(() => {
    const matched = joined.filter((r) => !r.asymmetric).length;
    const stOnly = joined.filter((r) => r.asymmetrySide === "storage_only").length;
    const opOnly = joined.filter((r) => r.asymmetrySide === "operator_only").length;
    return { total: joined.length, matched, stOnly, opOnly };
  }, [joined]);

  return (
    <div className="audit-page" data-testid="audit-page">
      <header className="audit-header">
        <h2>Audit</h2>
        <p className="muted">
          Two-sided view per SPEC §7. Patient-side rows from{" "}
          <code>OhdcService.AuditQuery</code>; operator-side rows from this
          browser's local audit log. JOIN is by <code>query_hash</code> with a
          fall-back match on <code>(action, ts ± 5s)</code>.
        </p>
      </header>

      <section className="audit-filters" aria-label="Audit filters">
        <ChipRow label="Actor">
          {(["all", "self", "grant"] as ActorChip[]).map((c) => (
            <Chip
              key={c}
              active={actorChip === c}
              onClick={() => setActorChip(c)}
              testId={`audit-actor-${c}`}
            >
              {c}
            </Chip>
          ))}
        </ChipRow>
        <ChipRow label="Op kind">
          {(["all", "read", "write", "pending", "case_op"] as OpChip[]).map((c) => (
            <Chip
              key={c}
              active={opChip === c}
              onClick={() => setOpChip(c)}
              testId={`audit-op-${c}`}
            >
              {c.replace("_", " ")}
            </Chip>
          ))}
        </ChipRow>
        <ChipRow label="Window">
          {WINDOWS.map((w) => (
            <Chip
              key={w.id}
              active={windowChip === w.id}
              onClick={() => setWindowChip(w.id)}
              testId={`audit-window-${w.id}`}
            >
              {w.label}
            </Chip>
          ))}
        </ChipRow>
        <div className="audit-actions">
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => void refresh()}>
            Refresh
          </button>
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            onClick={exportCsv}
            data-testid="audit-export-csv"
            disabled={joined.length === 0}
          >
            Export CSV
          </button>
        </div>
      </section>

      {loading && (
        <div className="empty">
          <p>Loading audit rows…</p>
        </div>
      )}

      {error && (
        <div className="empty">
          <h3>Could not load audit log</h3>
          <p className="muted mono" style={{ fontSize: 12 }}>
            {error}
          </p>
        </div>
      )}

      {!loading && !error && (
        <>
          <div className="audit-summary muted" style={{ marginBottom: 8 }}>
            <span>
              <strong>{counts.total}</strong> rows
            </span>
            {" · "}
            <span>{counts.matched} matched</span>
            {" · "}
            <span className={counts.stOnly > 0 ? "asymmetry-pill" : ""} data-testid="audit-asym-storage">
              {counts.stOnly} storage-only
            </span>
            {" · "}
            <span className={counts.opOnly > 0 ? "asymmetry-pill" : ""} data-testid="audit-asym-operator">
              {counts.opOnly} operator-only
            </span>
          </div>

          {joined.length === 0 ? (
            <div className="empty">
              <p>No audit rows match the current filters.</p>
            </div>
          ) : (
            <table className="audit-table" data-testid="audit-table">
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Actor</th>
                  <th>Op</th>
                  <th>Storage saw</th>
                  <th>We sent</th>
                  <th>Hash</th>
                </tr>
              </thead>
              <tbody>
                {joined.map((r) => (
                  <AuditRow key={r.id} row={r} />
                ))}
              </tbody>
            </table>
          )}
        </>
      )}
    </div>
  );
}

function AuditRow({ row }: { row: JoinedRow }) {
  const ts = new Date(row.tsMs).toLocaleString();
  const action = row.storage?.action ?? row.operator?.ohdcAction ?? "—";
  const queryKind = row.storage?.queryKind ?? row.operator?.queryKind ?? "";
  const actor = row.storage?.actorType ?? (row.operator?.grantUlid ? "grant" : "self");
  const rowsReturned = row.storage?.rowsReturned;
  const rowsFiltered = row.storage?.rowsFiltered;
  const opRowsReturned = row.operator?.rowsReturned;
  const filteredFlag = rowsFiltered != null && Number(rowsFiltered) > 0;
  const hashShort = row.joinHash ? row.joinHash.slice(0, 16) : "—";
  return (
    <tr
      data-testid={`audit-row-${row.id}`}
      data-asymmetry={row.asymmetrySide ?? "matched"}
      className={row.asymmetric ? "asymmetry-row" : ""}
    >
      <td className="mono" style={{ fontSize: 12 }}>{ts}</td>
      <td>{actor}</td>
      <td>
        <code>{action}</code>
        {queryKind && action !== queryKind && (
          <span className="muted"> / {queryKind}</span>
        )}
      </td>
      <td>
        {row.storage ? (
          <>
            <span className={`status-pill status-${row.storage.result}`}>{row.storage.result}</span>
            {" · "}
            <span>{rowsReturned != null ? `${Number(rowsReturned)} rows` : "—"}</span>
            {filteredFlag && (
              <>
                {" · "}
                <span className="muted" style={{ color: "var(--color-accent)" }}>
                  {Number(rowsFiltered)} filtered
                </span>
              </>
            )}
          </>
        ) : (
          <span className="asymmetry-badge" data-testid={`asym-storage-${row.id}`}>
            no storage row
          </span>
        )}
      </td>
      <td>
        {row.operator ? (
          <>
            <span className={`status-pill status-${row.operator.result}`}>{row.operator.result}</span>
            {" · "}
            <span>{opRowsReturned != null ? `${opRowsReturned} rows` : "—"}</span>
          </>
        ) : (
          <span className="asymmetry-badge" data-testid={`asym-operator-${row.id}`}>
            no operator row
          </span>
        )}
      </td>
      <td className="mono" style={{ fontSize: 12 }}>{hashShort}</td>
    </tr>
  );
}

// --- Tiny presentational helpers --------------------------------------------

function ChipRow({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="chip-row">
      <span className="chip-row-label muted">{label}</span>
      {children}
    </div>
  );
}

function Chip({
  active,
  onClick,
  children,
  testId,
}: {
  active: boolean;
  onClick: () => void;
  children: React.ReactNode;
  testId?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={`chip ${active ? "chip-active" : ""}`.trim()}
      data-testid={testId}
      data-active={active ? "true" : "false"}
    >
      {children}
    </button>
  );
}

function csvEscape(s: string): string {
  if (s.includes(",") || s.includes('"') || s.includes("\n")) {
    return `"${s.replace(/"/g, '""')}"`;
  }
  return s;
}
