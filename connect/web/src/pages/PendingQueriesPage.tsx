import { useMemo, useState } from "react";
import { useToast } from "../components/Toast";
import { fmtRelative, prettyEventType } from "../util";
import {
  approvePendingQueryById,
  bulkApprovePendingQueries,
  bulkRejectPendingQueries,
  getSnapshot,
  rejectPendingQueryById,
} from "../ohdc/store";
import { pendingQueriesIsMock, type PendingQuery } from "../ohdc/client";
import { useStoreVersion } from "../ohdc/useStore";

/**
 * Pending **read** queries queue — surfaces the `require_approval_per_query`
 * flag on grants. When that flag is set, every read RPC the grantee makes
 * lands here as a `pending_queries` row; the user approves (storage
 * executes the original query and returns rows to the grantee) or rejects
 * (grantee gets `OUT_OF_SCOPE`).
 *
 * Differences from `PendingPage` (which covers write-with-approval):
 *   - Each row shows the structured *query summary* (event types, time
 *     window) rather than a write payload.
 *   - No "trust forever" auto-approve action — that would defeat the point
 *     of `require_approval_per_query`. (A v0.x add-on could grow a
 *     per-pattern auto-approve rule against the grant; out of scope here.)
 *   - Multi-select + sticky bulk action bar at the bottom.
 *
 * Mock fallback: when the proto hasn't yet exposed the
 * `OhdcService.{List,Approve,Reject}PendingQuery` RPCs (the storage core
 * has the helpers, see storage `STATUS.md`), the `client.ts` shim falls
 * back to an in-memory store. We render a banner so reviewers know.
 */
export function PendingQueriesPage() {
  useStoreVersion();
  const snap = getSnapshot();
  const toast = useToast();
  const isMock = pendingQueriesIsMock();

  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [bulkRejectMode, setBulkRejectMode] = useState(false);
  const [bulkRejectReason, setBulkRejectReason] = useState("");

  const rows = snap.pendingQueries;

  const allSelected = useMemo(
    () => rows.length > 0 && rows.every((r) => selected.has(r.queryUlid)),
    [rows, selected],
  );

  const toggleAll = () => {
    if (allSelected) {
      setSelected(new Set());
    } else {
      setSelected(new Set(rows.map((r) => r.queryUlid)));
    }
  };

  const toggleOne = (id: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const onBulkApprove = async () => {
    setBusy(true);
    try {
      const ulids = [...selected];
      const { ok, failed } = await bulkApprovePendingQueries(ulids);
      toast.show(
        failed === 0
          ? `Approved ${ok} ${ok === 1 ? "query" : "queries"}.`
          : `Approved ${ok}, ${failed} failed.`,
        failed === 0 ? "success" : "error",
      );
      setSelected(new Set());
    } finally {
      setBusy(false);
    }
  };

  const onBulkReject = async () => {
    setBusy(true);
    try {
      const ulids = [...selected];
      const { ok, failed } = await bulkRejectPendingQueries(ulids, bulkRejectReason || undefined);
      toast.show(
        failed === 0
          ? `Rejected ${ok} ${ok === 1 ? "query" : "queries"}.`
          : `Rejected ${ok}, ${failed} failed.`,
        failed === 0 ? "success" : "error",
      );
      setSelected(new Set());
      setBulkRejectMode(false);
      setBulkRejectReason("");
    } finally {
      setBusy(false);
    }
  };

  return (
    <section data-testid="pending-queries-page">
      <header className="page-header">
        <div>
          <h1>Pending read queries</h1>
          <p>Read requests from grants flagged "approve every query."</p>
        </div>
      </header>

      {isMock ? (
        <div
          className="card"
          data-testid="pending-queries-mock-banner"
          style={{ borderLeft: "3px solid var(--color-warn)" }}
        >
          <div className="card-title">
            <h3 style={{ marginBottom: 0 }}>Demo data</h3>
            <span className="flag flag-warn">mock</span>
          </div>
          <p className="muted" style={{ marginTop: 8, marginBottom: 0, fontSize: 12 }}>
            Storage core exposes <code>list/approve/reject_pending_query</code> as
            internal helpers (see <code>storage/STATUS.md</code>), but the
            corresponding wire RPCs aren't in the proto yet — this page renders
            against an in-memory mock until they ship. The UI shape will not
            change when the wire path lights up.
          </p>
        </div>
      ) : null}

      {rows.length === 0 ? (
        <div className="empty">
          <p>No pending read queries. Things are quiet.</p>
        </div>
      ) : (
        <>
          <div
            className="card"
            style={{
              display: "flex",
              alignItems: "center",
              gap: 12,
              padding: "8px 12px",
              marginBottom: 12,
            }}
          >
            <label
              style={{ display: "inline-flex", alignItems: "center", gap: 6, cursor: "pointer" }}
            >
              <input
                type="checkbox"
                checked={allSelected}
                onChange={toggleAll}
                data-testid="select-all-queries"
              />
              <span className="muted" style={{ fontSize: 12 }}>
                {selected.size > 0 ? `${selected.size} selected` : "Select all"}
              </span>
            </label>
            <span className="muted" style={{ fontSize: 12, marginLeft: "auto" }}>
              {rows.length} pending
            </span>
          </div>

          {rows.map((p) => (
            <PendingQueryCard
              key={p.queryUlid}
              row={p}
              selected={selected.has(p.queryUlid)}
              onToggle={() => toggleOne(p.queryUlid)}
              onAct={(msg, kind) => toast.show(msg, kind)}
            />
          ))}
        </>
      )}

      {selected.size > 0 ? (
        <div
          className="card"
          data-testid="bulk-action-bar"
          style={{
            position: "sticky",
            bottom: 0,
            marginTop: 16,
            padding: 12,
            background: "var(--color-surface-2)",
            borderTop: "1px solid var(--color-border-strong)",
          }}
        >
          {bulkRejectMode ? (
            <>
              <label className="field">
                Reason for rejecting {selected.size}{" "}
                {selected.size === 1 ? "query" : "queries"} (optional)
                <textarea
                  value={bulkRejectReason}
                  onChange={(ev) => setBulkRejectReason(ev.target.value)}
                  placeholder="e.g. out of scope for this grant"
                />
              </label>
              <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
                <button
                  className="btn"
                  disabled={busy}
                  onClick={() => {
                    setBulkRejectMode(false);
                    setBulkRejectReason("");
                  }}
                >
                  Cancel
                </button>
                <button
                  className="btn btn-danger"
                  disabled={busy}
                  onClick={onBulkReject}
                  data-testid="bulk-reject-confirm"
                >
                  Reject {selected.size}
                </button>
              </div>
            </>
          ) : (
            <div style={{ display: "flex", gap: 8, flexWrap: "wrap", alignItems: "center" }}>
              <span style={{ marginRight: "auto", fontSize: 13 }}>
                {selected.size} {selected.size === 1 ? "query" : "queries"} selected
              </span>
              <button
                className="btn btn-accent"
                disabled={busy}
                onClick={onBulkApprove}
                data-testid="bulk-approve"
              >
                Approve {selected.size}
              </button>
              <button
                className="btn btn-danger"
                disabled={busy}
                onClick={() => setBulkRejectMode(true)}
                data-testid="bulk-reject"
              >
                Reject {selected.size}…
              </button>
            </div>
          )}
        </div>
      ) : null}
    </section>
  );
}

function PendingQueryCard({
  row,
  selected,
  onToggle,
  onAct,
}: {
  row: PendingQuery;
  selected: boolean;
  onToggle: () => void;
  onAct: (msg: string, kind: "success" | "error") => void;
}) {
  const [busy, setBusy] = useState(false);
  const [showReject, setShowReject] = useState(false);
  const [reason, setReason] = useState("");

  const expiresInMs = row.expiresAtMs - Date.now();
  const expiresLabel =
    expiresInMs <= 0
      ? "expired"
      : expiresInMs < 60_000
      ? `${Math.round(expiresInMs / 1000)}s left`
      : expiresInMs < 3_600_000
      ? `${Math.round(expiresInMs / 60_000)}m left`
      : `${Math.round(expiresInMs / 3_600_000)}h left`;

  return (
    <div className="card" data-testid="pending-query-card">
      <div className="card-title" style={{ alignItems: "flex-start", gap: 12 }}>
        <input
          type="checkbox"
          checked={selected}
          onChange={onToggle}
          aria-label={`Select query ${row.queryUlid}`}
          style={{ marginTop: 4 }}
          data-testid={`select-${row.queryUlid}`}
        />
        <div style={{ flex: 1, minWidth: 0 }}>
          <h3 style={{ marginBottom: 2 }}>{row.grantLabel || "(unlabelled grant)"}</h3>
          <div className="muted" style={{ fontSize: 12 }}>
            grant <span className="mono">{row.grantUlid.slice(0, 8)}…</span> ·{" "}
            requested {fmtRelative(row.requestedAtMs)} · {row.queryKind}
          </div>
        </div>
        <span className={`flag ${expiresInMs <= 60_000 ? "flag-warn" : "flag-active"}`}>
          {expiresLabel}
        </span>
      </div>

      <table className="data-table" style={{ marginBottom: 12 }}>
        <tbody>
          <tr>
            <th style={{ textAlign: "left", width: "30%" }}>Wants to read</th>
            <td>
              {row.summary.eventTypes.length > 0
                ? row.summary.eventTypes.map(prettyEventType).join(", ")
                : "(any event type)"}
            </td>
          </tr>
          <tr>
            <th style={{ textAlign: "left" }}>Time window</th>
            <td>{formatWindow(row.summary.fromMs, row.summary.toMs)}</td>
          </tr>
          {row.summary.hint ? (
            <tr>
              <th style={{ textAlign: "left" }}>Notes</th>
              <td>{row.summary.hint}</td>
            </tr>
          ) : null}
        </tbody>
      </table>

      {showReject ? (
        <>
          <label className="field">
            Reason (optional)
            <textarea
              value={reason}
              onChange={(ev) => setReason(ev.target.value)}
              placeholder="e.g. out of scope, looks like a probe"
            />
          </label>
          <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
            <button className="btn" disabled={busy} onClick={() => setShowReject(false)}>
              Cancel
            </button>
            <button
              className="btn btn-danger"
              disabled={busy}
              onClick={async () => {
                setBusy(true);
                try {
                  await rejectPendingQueryById(row.queryUlid, reason || undefined);
                  onAct("Rejected.", "success");
                } catch (err) {
                  onAct(`Reject failed: ${(err as Error).message ?? String(err)}`, "error");
                } finally {
                  setBusy(false);
                  setShowReject(false);
                }
              }}
              data-testid={`reject-${row.queryUlid}`}
            >
              Confirm reject
            </button>
          </div>
        </>
      ) : (
        <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
          <button
            className="btn btn-accent"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await approvePendingQueryById(row.queryUlid);
                onAct("Approved.", "success");
              } catch (err) {
                onAct(`Approve failed: ${(err as Error).message ?? String(err)}`, "error");
              } finally {
                setBusy(false);
              }
            }}
            data-testid={`approve-${row.queryUlid}`}
          >
            Approve
          </button>
          <button
            className="btn btn-danger"
            disabled={busy}
            onClick={() => setShowReject(true)}
          >
            Reject
          </button>
        </div>
      )}
    </div>
  );
}

function formatWindow(fromMs: number | null, toMs: number | null): string {
  if (fromMs == null && toMs == null) return "all time";
  const now = Date.now();
  if (fromMs != null && toMs == null) {
    const ago = now - fromMs;
    if (ago > 0 && ago < 86_400_000 * 365) {
      const days = Math.round(ago / 86_400_000);
      if (days <= 1) return "last 24 hours";
      if (days <= 7) return `last ${days} days`;
      if (days <= 31) return `last ${days} days`;
      const months = Math.round(days / 30);
      return `last ${months} months`;
    }
    return `since ${new Date(fromMs).toISOString().slice(0, 10)}`;
  }
  if (fromMs == null && toMs != null) {
    return `up to ${new Date(toMs).toISOString().slice(0, 10)}`;
  }
  return `${new Date(fromMs!).toISOString().slice(0, 10)} → ${new Date(toMs!)
    .toISOString()
    .slice(0, 10)}`;
}
