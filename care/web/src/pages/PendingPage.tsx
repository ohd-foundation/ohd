// PendingPage.tsx — operator-side pending queue for the active patient.
//
// Per care/SPEC.md §6.1, every operator submission against a grant with
// approval_mode=`always` (or a non-allowlisted event_type under
// `auto_for_event_types`) lands in the patient's pending queue until the
// patient approves. Care v0.x lets the operator inspect that queue (read-
// only on the patient's side); v0.x extends to bulk approve / reject for
// trusted deployments where the operator is empowered to pre-approve their
// own writes (e.g., established care relationship; the patient still gets
// the audit row).
//
// UX (per the deliverables brief):
// - Multi-select mode: checkbox per row + "Select all visible".
// - Sticky toolbar appears when ≥1 selected: "Approve selected (N)" /
//   "Reject selected (N)" / "Cancel".
// - Confirmation dialog: count + sample of submitter names.
// - Each item iterates through OhdcService.{ApprovePending,RejectPending}
//   with progress feedback (per-item success/error).
// - Mid-batch error: pause, show partial-progress summary, ask
//   "continue / abort".
// - Success toast with count.
// - "Trust forever" path: if all selected items share the same event_type,
//   offer "Approve all and add `<event_type>` to auto-approval".

import { useCallback, useEffect, useMemo, useState } from "react";
import {
  approvePending,
  listPending,
  rejectPending,
  ulidToCrockford,
  type PendingEvent,
} from "../ohdc/client";
import { useToast } from "../components/Toast";

interface BatchProgress {
  total: number;
  succeeded: string[]; // pending ulids
  failed: { ulid: string; error: string }[];
  /** Indices not yet attempted. Set when a batch is paused mid-run. */
  remaining: string[];
  /** "approve" or "reject" — what we're processing. */
  action: "approve" | "reject";
  /** True when the user paused mid-batch via the error dialog. */
  paused: boolean;
}

export function PendingPage() {
  const toast = useToast();
  const [pending, setPending] = useState<PendingEvent[]>([]);
  const [loading, setLoading] = useState<boolean>(true);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirm, setConfirm] = useState<
    | null
    | {
        action: "approve" | "reject";
        ulids: string[];
        reason?: string;
        addAutoApproveEventType?: string;
      }
  >(null);
  const [progress, setProgress] = useState<BatchProgress | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const items = await listPending();
      setPending(items);
    } catch (err) {
      setError((err as Error).message ?? String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const visibleUlids = useMemo(
    () =>
      pending
        .map((p) => (p.ulid ? ulidToCrockford(p.ulid.bytes) : null))
        .filter((u): u is string => u !== null),
    [pending],
  );

  const toggleOne = useCallback((ulid: string) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(ulid)) next.delete(ulid);
      else next.add(ulid);
      return next;
    });
  }, []);

  const selectAllVisible = useCallback(() => {
    setSelected(new Set(visibleUlids));
  }, [visibleUlids]);

  const clearSelection = useCallback(() => {
    setSelected(new Set());
  }, []);

  // If every selected item is the same event_type, offer the "trust forever"
  // shortcut. Per SPEC §6.1's `auto_for_event_types` mode.
  const sharedEventType = useMemo<string | null>(() => {
    if (selected.size === 0) return null;
    const types = new Set<string>();
    for (const p of pending) {
      const ulid = p.ulid ? ulidToCrockford(p.ulid.bytes) : null;
      if (ulid && selected.has(ulid)) {
        if (p.event?.eventType) types.add(p.event.eventType);
      }
    }
    return types.size === 1 ? Array.from(types)[0] : null;
  }, [selected, pending]);

  const sampleSubmitterLabels = useCallback(
    (ulids: string[]): string[] => {
      const labels: string[] = [];
      for (const p of pending) {
        const u = p.ulid ? ulidToCrockford(p.ulid.bytes) : null;
        if (!u || !ulids.includes(u)) continue;
        const ev = p.event;
        if (ev) {
          // Operator-readable summary: event_type + submitted_at.
          const ts = new Date(Number(p.submittedAtMs)).toISOString().slice(0, 19);
          labels.push(`${ev.eventType} @ ${ts}`);
        }
        if (labels.length >= 5) break;
      }
      return labels;
    },
    [pending],
  );

  const runBatch = useCallback(
    async (
      action: "approve" | "reject",
      ulids: string[],
      opts: { reason?: string; addAutoApproveEventType?: string },
    ) => {
      // Initialize progress with the full queue; we drain it one by one so
      // the UI can show "n of N done" even if the network is slow.
      setProgress({
        total: ulids.length,
        succeeded: [],
        failed: [],
        remaining: [...ulids],
        action,
        paused: false,
      });

      // Take the auto-approve shortcut on the FIRST approval call only —
      // storage flips the grant's allowlist on that single call; subsequent
      // approvals don't need to repeat it.
      let firstApproveAutoFlag = action === "approve" && !!opts.addAutoApproveEventType;

      for (let i = 0; i < ulids.length; i++) {
        const u = ulids[i];
        try {
          if (action === "approve") {
            await approvePending(u, firstApproveAutoFlag);
            firstApproveAutoFlag = false;
          } else {
            await rejectPending(u, opts.reason);
          }
          setProgress((p) =>
            p
              ? {
                  ...p,
                  succeeded: [...p.succeeded, u],
                  remaining: p.remaining.filter((x) => x !== u),
                }
              : p,
          );
        } catch (err) {
          const message = (err as Error).message ?? String(err);
          setProgress((p) =>
            p
              ? {
                  ...p,
                  failed: [...p.failed, { ulid: u, error: message }],
                  remaining: p.remaining.filter((x) => x !== u),
                  paused: true,
                }
              : p,
          );
          // Stop the loop on first error; the operator decides via the
          // "continue / abort" affordance.
          return;
        }
      }
    },
    [],
  );

  const onContinueAfterError = useCallback(() => {
    if (!progress) return;
    const { remaining, action } = progress;
    setProgress((p) => (p ? { ...p, paused: false } : p));
    void runBatch(action, remaining, {
      // Continue path doesn't re-attempt the auto-approve flag — it
      // either stuck on the first call or never will.
      reason: undefined,
      addAutoApproveEventType: undefined,
    });
  }, [progress, runBatch]);

  const onAbortBatch = useCallback(() => {
    if (!progress) return;
    const { succeeded, failed } = progress;
    toast.show(
      `Batch ${progress.action} aborted: ${succeeded.length} done, ${failed.length} failed, ${progress.remaining.length} skipped.`,
    );
    setProgress(null);
    setSelected(new Set());
    void refresh();
  }, [progress, refresh, toast]);

  const onCloseSummary = useCallback(() => {
    if (!progress) return;
    const { succeeded, failed, action } = progress;
    if (failed.length === 0) {
      toast.show(
        `${action === "approve" ? "Approved" : "Rejected"} ${succeeded.length} submission${succeeded.length === 1 ? "" : "s"}.`,
        "success",
      );
    } else {
      toast.show(
        `${action === "approve" ? "Approved" : "Rejected"} ${succeeded.length}, failed ${failed.length}.`,
      );
    }
    setProgress(null);
    setSelected(new Set());
    void refresh();
  }, [progress, refresh, toast]);

  const startApprove = useCallback(
    (addAutoApproveEventType?: string) => {
      const ulids = Array.from(selected);
      if (ulids.length === 0) return;
      setConfirm({
        action: "approve",
        ulids,
        addAutoApproveEventType,
      });
    },
    [selected],
  );

  const startReject = useCallback(() => {
    const ulids = Array.from(selected);
    if (ulids.length === 0) return;
    setConfirm({ action: "reject", ulids });
  }, [selected]);

  const onConfirm = useCallback(async () => {
    if (!confirm) return;
    const c = confirm;
    setConfirm(null);
    await runBatch(c.action, c.ulids, {
      reason: c.reason,
      addAutoApproveEventType: c.addAutoApproveEventType,
    });
  }, [confirm, runBatch]);

  if (loading) {
    return (
      <div className="empty">
        <p>Loading pending submissions…</p>
      </div>
    );
  }

  if (error) {
    return (
      <div className="empty">
        <h3>Could not load pending submissions</h3>
        <p className="muted mono" style={{ fontSize: 12 }}>
          {error}
        </p>
        <button type="button" className="btn" onClick={() => void refresh()}>
          Retry
        </button>
      </div>
    );
  }

  return (
    <div data-testid="pending-page" className="pending-page">
      <header className="pending-header">
        <h2>Pending submissions</h2>
        <p className="muted">
          Submissions queued for patient review under your grant. Per
          SPEC §6.1: the patient sees these in OHD Connect; the operator
          can pre-approve here when the grant policy allows it.
        </p>
      </header>

      {pending.length === 0 ? (
        <div className="empty">
          <p>No pending submissions for this patient.</p>
        </div>
      ) : (
        <>
          <div className="pending-toolbar-row">
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={selectAllVisible}
              data-testid="select-all"
            >
              Select all visible ({visibleUlids.length})
            </button>
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={clearSelection}
              disabled={selected.size === 0}
            >
              Clear selection
            </button>
            <span className="muted">
              {selected.size} of {pending.length} selected
            </span>
          </div>

          <table className="pending-table">
            <thead>
              <tr>
                <th aria-label="select" />
                <th>Submitted</th>
                <th>Type</th>
                <th>Channels</th>
                <th>Expires</th>
              </tr>
            </thead>
            <tbody>
              {pending.map((p) => {
                const ulid = p.ulid ? ulidToCrockford(p.ulid.bytes) : "";
                if (!ulid) return null;
                const submitted = new Date(Number(p.submittedAtMs)).toLocaleString();
                const expires =
                  Number(p.expiresAtMs) > 0
                    ? new Date(Number(p.expiresAtMs)).toLocaleString()
                    : "—";
                const channels = (p.event?.channels ?? [])
                  .map((c) => c.channelPath)
                  .join(", ");
                return (
                  <tr key={ulid} data-testid={`pending-row-${ulid}`}>
                    <td>
                      <input
                        type="checkbox"
                        checked={selected.has(ulid)}
                        onChange={() => toggleOne(ulid)}
                        aria-label={`select pending ${ulid}`}
                      />
                    </td>
                    <td>{submitted}</td>
                    <td>{p.event?.eventType ?? "—"}</td>
                    <td className="mono" style={{ fontSize: 12 }}>
                      {channels || "—"}
                    </td>
                    <td>{expires}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>

          {selected.size > 0 && (
            <div className="pending-sticky-toolbar" role="toolbar" data-testid="bulk-toolbar">
              <button
                type="button"
                className="btn btn-primary"
                onClick={() => startApprove()}
                data-testid="bulk-approve"
              >
                Approve selected ({selected.size})
              </button>
              <button
                type="button"
                className="btn"
                onClick={startReject}
                data-testid="bulk-reject"
              >
                Reject selected ({selected.size})
              </button>
              {sharedEventType && (
                <button
                  type="button"
                  className="btn btn-ghost"
                  onClick={() => startApprove(sharedEventType)}
                  data-testid="bulk-approve-trust"
                  title={`Approve all and add ${sharedEventType} to auto-approval for this grant.`}
                >
                  Approve & trust «{sharedEventType}»
                </button>
              )}
              <button
                type="button"
                className="btn btn-ghost"
                onClick={clearSelection}
                data-testid="bulk-cancel"
              >
                Cancel
              </button>
            </div>
          )}
        </>
      )}

      {confirm && (
        <ConfirmDialog
          confirm={confirm}
          sampleLabels={sampleSubmitterLabels(confirm.ulids)}
          onCancel={() => setConfirm(null)}
          onConfirm={onConfirm}
          onUpdateReason={(reason) =>
            setConfirm((c) => (c ? { ...c, reason } : c))
          }
        />
      )}

      {progress && (
        <ProgressOverlay
          progress={progress}
          onContinue={onContinueAfterError}
          onAbort={onAbortBatch}
          onClose={onCloseSummary}
        />
      )}
    </div>
  );
}

// --- Subcomponents -----------------------------------------------------------

function ConfirmDialog({
  confirm,
  sampleLabels,
  onCancel,
  onConfirm,
  onUpdateReason,
}: {
  confirm: {
    action: "approve" | "reject";
    ulids: string[];
    reason?: string;
    addAutoApproveEventType?: string;
  };
  sampleLabels: string[];
  onCancel: () => void;
  onConfirm: () => void;
  onUpdateReason: (reason: string) => void;
}) {
  return (
    <div className="modal-backdrop" data-testid="bulk-confirm">
      <div className="modal" role="dialog" aria-labelledby="bulk-confirm-title">
        <h3 id="bulk-confirm-title">
          {confirm.action === "approve" ? "Approve" : "Reject"}{" "}
          {confirm.ulids.length} submission
          {confirm.ulids.length === 1 ? "" : "s"}?
        </h3>
        {confirm.addAutoApproveEventType && (
          <p>
            <strong>Trust forever:</strong> «{confirm.addAutoApproveEventType}»
            will be added to this grant's auto-approval allowlist; future
            submissions of this type will commit without review. Per SPEC
            §6.1 (<code>auto_for_event_types</code>).
          </p>
        )}
        <ul className="muted mono" style={{ fontSize: 12 }}>
          {sampleLabels.map((s, i) => (
            <li key={i}>{s}</li>
          ))}
          {confirm.ulids.length > sampleLabels.length && (
            <li>… and {confirm.ulids.length - sampleLabels.length} more</li>
          )}
        </ul>
        {confirm.action === "reject" && (
          <label className="form-row">
            <span>Reason (optional)</span>
            <input
              type="text"
              value={confirm.reason ?? ""}
              onChange={(e) => onUpdateReason(e.target.value)}
              placeholder="e.g. duplicate of <ulid>; submitter please re-issue"
              data-testid="bulk-reason"
            />
          </label>
        )}
        <div className="modal-actions">
          <button type="button" className="btn btn-ghost" onClick={onCancel}>
            Cancel
          </button>
          <button
            type="button"
            className={confirm.action === "approve" ? "btn btn-primary" : "btn btn-danger"}
            onClick={onConfirm}
            data-testid="bulk-confirm-go"
          >
            {confirm.action === "approve" ? "Approve" : "Reject"}{" "}
            {confirm.ulids.length}
          </button>
        </div>
      </div>
    </div>
  );
}

function ProgressOverlay({
  progress,
  onContinue,
  onAbort,
  onClose,
}: {
  progress: BatchProgress;
  onContinue: () => void;
  onAbort: () => void;
  onClose: () => void;
}) {
  const done = progress.succeeded.length + progress.failed.length;
  const remaining = progress.remaining.length;
  const isRunning = remaining > 0 && !progress.paused;
  const isPaused = progress.paused && remaining > 0;
  const isComplete = remaining === 0;
  return (
    <div className="modal-backdrop" data-testid="bulk-progress">
      <div className="modal" role="dialog">
        <h3>
          {progress.action === "approve" ? "Approving" : "Rejecting"}{" "}
          submissions… {done} / {progress.total}
        </h3>
        <progress max={progress.total} value={done} />
        <ul style={{ marginTop: 8 }}>
          <li>{progress.succeeded.length} succeeded</li>
          <li>{progress.failed.length} failed</li>
          <li>{remaining} remaining</li>
        </ul>
        {progress.failed.length > 0 && (
          <details>
            <summary>Errors ({progress.failed.length})</summary>
            <ul className="mono" style={{ fontSize: 12 }}>
              {progress.failed.map((f, i) => (
                <li key={i}>
                  <code>{f.ulid}</code> — {f.error}
                </li>
              ))}
            </ul>
          </details>
        )}
        <div className="modal-actions">
          {isRunning && <p className="muted">Working…</p>}
          {isPaused && (
            <>
              <button
                type="button"
                className="btn btn-ghost"
                onClick={onAbort}
                data-testid="bulk-abort"
              >
                Abort
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={onContinue}
                data-testid="bulk-continue"
              >
                Continue with {remaining} remaining
              </button>
            </>
          )}
          {isComplete && (
            <button
              type="button"
              className="btn btn-primary"
              onClick={onClose}
              data-testid="bulk-done"
            >
              Done
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
