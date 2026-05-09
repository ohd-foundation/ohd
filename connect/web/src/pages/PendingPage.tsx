import { useState } from "react";
import { useToast } from "../components/Toast";
import { fmtRelative, prettyEventType } from "../util";
import { approvePendingById, getSnapshot, rejectPendingById } from "../ohdc/store";
import { ulidToCrockford } from "../ohdc/client";
import { useStoreVersion } from "../ohdc/useStore";
import type { PendingEvent } from "../ohdc/client";

/**
 * Pending review queue. Each item shows the submitter (grant ULID, since
 * `submitting_grant_ulid` is what storage exposes), a structured preview of
 * the proposed event, and three actions:
 *   - Approve
 *   - Approve and trust this event-type from this grant going forward
 *   - Reject (with optional free-text reason)
 *
 * Bulk select is v0.x; v0.1 acts on one row at a time.
 */
export function PendingPage() {
  useStoreVersion();
  const snap = getSnapshot();
  const toast = useToast();

  return (
    <section data-testid="pending-page">
      <header className="page-header">
        <div>
          <h1>Pending writes</h1>
          <p>Writes from grant-holders awaiting your approval. Read-side queue lives at <a href="/pending-queries">Pending reads</a>.</p>
        </div>
      </header>

      {snap.pending.length === 0 ? (
        <div className="empty">
          <p>No pending submissions. Things are quiet.</p>
        </div>
      ) : (
        snap.pending.map((p) => <PendingCard key={ulidToCrockford(p.ulid?.bytes)} pending={p} onAct={(msg, kind) => toast.show(msg, kind)} />)
      )}
    </section>
  );
}

function PendingCard({ pending, onAct }: { pending: PendingEvent; onAct: (msg: string, kind: "success" | "error") => void }) {
  const [busy, setBusy] = useState(false);
  const [showReject, setShowReject] = useState(false);
  const [reason, setReason] = useState("");

  const ulid = ulidToCrockford(pending.ulid?.bytes);
  const grantUlid = ulidToCrockford(pending.submittingGrantUlid?.bytes);
  const e = pending.event;

  return (
    <div className="card" data-testid="pending-card">
      <div className="card-title">
        <div>
          <h3>{e ? prettyEventType(e.eventType) : "(unknown event)"}</h3>
          <div className="muted" style={{ fontSize: 12, marginTop: 2 }}>
            from grant <span className="mono">{grantUlid.slice(0, 8)}…</span> · submitted{" "}
            {fmtRelative(Number(pending.submittedAtMs))}
          </div>
        </div>
        <span className="flag flag-active">{pending.status}</span>
      </div>

      {e ? (
        <table className="data-table" style={{ marginBottom: 12 }}>
          <thead>
            <tr>
              <th>Channel</th>
              <th className="num">Value</th>
            </tr>
          </thead>
          <tbody>
            {e.channels.map((c, idx) => (
              <tr key={idx}>
                <td className="mono">{c.channelPath}</td>
                <td className="num">{renderChannel(c)}</td>
              </tr>
            ))}
            {e.notes ? (
              <tr>
                <td className="mono">notes</td>
                <td>{e.notes}</td>
              </tr>
            ) : null}
          </tbody>
        </table>
      ) : null}

      {showReject ? (
        <>
          <label className="field">
            Reason (optional)
            <textarea value={reason} onChange={(ev) => setReason(ev.target.value)} placeholder="e.g. wrong patient, value clearly wrong" />
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
                  await rejectPendingById(ulid, reason || undefined);
                  onAct("Rejected.", "success");
                } catch (err) {
                  onAct(`Reject failed: ${(err as Error).message ?? String(err)}`, "error");
                } finally {
                  setBusy(false);
                  setShowReject(false);
                }
              }}
              data-testid={`reject-${ulid}`}
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
                await approvePendingById(ulid, false);
                onAct("Approved.", "success");
              } catch (err) {
                onAct(`Approve failed: ${(err as Error).message ?? String(err)}`, "error");
              } finally {
                setBusy(false);
              }
            }}
            data-testid={`approve-${ulid}`}
          >
            Approve
          </button>
          <button
            className="btn"
            disabled={busy}
            onClick={async () => {
              setBusy(true);
              try {
                await approvePendingById(ulid, true);
                onAct("Approved & trusted this type from this grant.", "success");
              } catch (err) {
                onAct(`Approve failed: ${(err as Error).message ?? String(err)}`, "error");
              } finally {
                setBusy(false);
              }
            }}
          >
            Approve & trust type
          </button>
          <button className="btn btn-danger" disabled={busy} onClick={() => setShowReject(true)}>
            Reject
          </button>
        </div>
      )}
    </div>
  );
}

function renderChannel(c: NonNullable<PendingEvent["event"]>["channels"][number]): string {
  switch (c.value.case) {
    case "realValue":
      return c.value.value.toFixed(2);
    case "intValue":
      return String(c.value.value);
    case "boolValue":
      return c.value.value ? "true" : "false";
    case "textValue":
      return c.value.value;
    case "enumOrdinal":
      return `[#${c.value.value}]`;
    default:
      return "—";
  }
}
