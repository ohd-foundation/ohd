import { useStoreVersion } from "../../ohdc/useStore";
import { closeCaseById, getSnapshot } from "../../ohdc/store";
import { useToast } from "../../components/Toast";
import { ulidToCrockford } from "../../ohdc/client";
import { fmtDate, fmtRelative } from "../../util";

/**
 * Settings → Cases.
 *
 * Lists all cases (active + closed). For active cases, exposes:
 *   - Force-close (calls `Cases.CloseCase` with a reason).
 *   - "Issue retrospective grant" — surfaced as TBD until storage Cases CRUD
 *     ships. The proto exists; the server returns Unimplemented today
 *     (storage/STATUS.md "Cases CRUD + scope resolution: v1.x").
 *
 * Auto-granted (timeout-default-allow) entries get a distinct visual badge
 * per the designer-handoff doc.
 */
export function CasesSettingsPage() {
  useStoreVersion();
  const snap = getSnapshot();
  const toast = useToast();

  const active = snap.cases.filter((c) => !c.endedAtMs);
  const closed = snap.cases.filter((c) => c.endedAtMs);

  return (
    <div data-testid="settings-cases">
      <div className="banner info">
        Cases group reads/writes that belong together (an emergency, a hospital admission,
        a clinic visit). Storage's Cases CRUD ships in v1.x; today this view reads what's
        there but the action buttons may return <code>Unimplemented</code>.
      </div>

      <div className="card">
        <div className="card-title">
          <h3>Active cases ({active.length})</h3>
        </div>
        {active.length === 0 ? (
          <div className="empty" style={{ padding: 16 }}>
            No active cases.
          </div>
        ) : (
          active.map((c) => {
            const ulid = ulidToCrockford(c.ulid?.bytes);
            return (
              <div key={ulid} className="toggle-row" style={{ alignItems: "flex-start" }}>
                <div className="copy">
                  <span className="title">
                    {c.caseLabel ?? `${c.caseType} case`}{" "}
                    <span className="badge badge-active" style={{ marginLeft: 6 }}>
                      {c.caseType}
                    </span>
                  </span>
                  <span className="sub">
                    Started {fmtDate(Number(c.startedAtMs))} ({fmtRelative(Number(c.startedAtMs))}) ·{" "}
                    last activity {fmtRelative(Number(c.lastActivityAtMs))}
                    <br />
                    ULID <span className="mono">{ulid.slice(0, 8)}…</span>
                    {c.openingAuthorityGrantUlid ? (
                      <>
                        {" "}· authority grant <span className="mono">{ulidToCrockford(c.openingAuthorityGrantUlid.bytes).slice(0, 8)}…</span>
                      </>
                    ) : null}
                  </span>
                </div>
                <div style={{ display: "flex", gap: 8, flexShrink: 0 }}>
                  <button
                    className="btn btn-sm"
                    onClick={() => alert("Retrospective grants ship with storage Cases CRUD — see STATUS.md.")}
                  >
                    Issue retro grant (TBD)
                  </button>
                  <button
                    className="btn btn-sm btn-danger"
                    onClick={async () => {
                      const reason = prompt(`Force-close case ${ulid.slice(0, 8)}? Optional reason:`) ?? undefined;
                      try {
                        await closeCaseById(ulid, reason || undefined);
                        toast.show("Case force-closed.", "success");
                      } catch (err) {
                        toast.show(`Close failed: ${(err as Error).message ?? String(err)}`, "error");
                      }
                    }}
                  >
                    Force close
                  </button>
                </div>
              </div>
            );
          })
        )}
      </div>

      <div className="card">
        <div className="card-title">
          <h3>Closed cases ({closed.length})</h3>
        </div>
        {closed.length === 0 ? (
          <div className="empty" style={{ padding: 16 }}>
            No closed cases.
          </div>
        ) : (
          <table className="data-table">
            <thead>
              <tr>
                <th>Type</th>
                <th>Label</th>
                <th>Started</th>
                <th>Ended</th>
                <th>ULID</th>
              </tr>
            </thead>
            <tbody>
              {closed.map((c) => {
                const ulid = ulidToCrockford(c.ulid?.bytes);
                return (
                  <tr key={ulid}>
                    <td>{c.caseType}</td>
                    <td>{c.caseLabel ?? "—"}</td>
                    <td className="mono">{fmtDate(Number(c.startedAtMs))}</td>
                    <td className="mono">{c.endedAtMs ? fmtDate(Number(c.endedAtMs)) : "—"}</td>
                    <td className="mono">{ulid.slice(0, 8)}…</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
