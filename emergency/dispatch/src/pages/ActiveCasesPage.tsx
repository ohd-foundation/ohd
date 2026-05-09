import { useEffect, useMemo, useState } from "react";
import { Column, DataTable } from "../components/DataTable";
import { MetricTile } from "../components/MetricTile";
import { StatusChip } from "../components/StatusChip";
import { TimelineFeed } from "../components/TimelineFeed";
import { fetchCaseRow, forceCloseCase, getCases, getTimeline, isMockMode } from "../mock/store";
import { usePoll, useStoreVersion } from "../ohdc/useStore";
import type { CaseRow, TimelineRow } from "../types";
import { fmtClock, fmtElapsed, fmtStamp, shortUlid } from "../util";

/**
 * Default page. Live case board: table on the left, lightweight map
 * placeholder + metric strip up top. Click a row to open the detail
 * drawer (right column).
 */
export function ActiveCasesPage() {
  useStoreVersion();
  usePoll(5000);
  const cases = getCases();
  const [filter, setFilter] = useState("");
  const [showClosed, setShowClosed] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [drawerCase, setDrawerCase] = useState<CaseRow | null>(null);
  const [timeline, setTimeline] = useState<TimelineRow[]>([]);

  // Re-fetch the row + its timeline when a row is clicked.
  useEffect(() => {
    let alive = true;
    if (!selected) {
      setDrawerCase(null);
      setTimeline([]);
      return;
    }
    void fetchCaseRow(selected).then((row) => {
      if (!alive) return;
      setDrawerCase(row);
    });
    setTimeline(getTimeline(selected));
    return () => {
      alive = false;
    };
  }, [selected]);

  const filteredCases = useMemo(
    () => (showClosed ? cases : cases.filter((c) => c.status !== "closed")),
    [cases, showClosed],
  );

  const metrics = useMemo(() => {
    const open = cases.filter((c) => c.status === "open").length;
    const handoff = cases.filter((c) => c.status === "handoff").length;
    const closedToday = cases.filter((c) => c.status === "closed").length;
    return { open, handoff, closedToday, total: cases.length };
  }, [cases]);

  const columns: Column<CaseRow>[] = [
    {
      key: "case_ulid",
      header: "Case",
      width: "140px",
      cell: (r) => <span className="mono">{shortUlid(r.case_ulid)}</span>,
      sort: (a, b) => a.case_ulid.localeCompare(b.case_ulid),
    },
    {
      key: "patient_label",
      header: "Patient",
      width: "120px",
      cell: (r) => <strong>{r.patient_label}</strong>,
      sort: (a, b) => a.patient_label.localeCompare(b.patient_label),
    },
    {
      key: "status",
      header: "Status",
      width: "100px",
      cell: (r) => <StatusChip status={r.status} />,
      sort: (a, b) => a.status.localeCompare(b.status),
    },
    {
      key: "case_type",
      header: "Type",
      width: "110px",
      sort: (a, b) => a.case_type.localeCompare(b.case_type),
    },
    {
      key: "opened_at",
      header: "Opened",
      width: "120px",
      cell: (r) => <span className="mono">{fmtClock(r.opened_at_ms)}</span>,
      sort: (a, b) => a.opened_at_ms - b.opened_at_ms,
      align: "num",
    },
    {
      key: "elapsed",
      header: "Elapsed",
      width: "90px",
      cell: (r) => (
        <span className="mono">{fmtElapsed(Date.now() - r.opened_at_ms)}</span>
      ),
      sort: (a, b) => a.opened_at_ms - b.opened_at_ms,
      align: "num",
    },
    {
      key: "last_activity",
      header: "Last event",
      width: "120px",
      cell: (r) => (
        <span className="mono">{fmtElapsed(Date.now() - r.last_activity_ms)} ago</span>
      ),
      sort: (a, b) => a.last_activity_ms - b.last_activity_ms,
      align: "num",
    },
    {
      key: "responder",
      header: "Responder",
      cell: (r) => r.opening_responder,
      sort: (a, b) => a.opening_responder.localeCompare(b.opening_responder),
    },
  ];

  return (
    <div className="page" data-testid="active-cases-page">
      <header className="page-head">
        <div>
          <h1>Active cases</h1>
          <p className="muted">Live case board · refreshes every 5s.</p>
        </div>
        <div className="page-head-actions">
          <label className="checkbox">
            <input
              type="checkbox"
              checked={showClosed}
              onChange={(e) => setShowClosed(e.target.checked)}
            />
            include closed
          </label>
          <input
            type="search"
            className="input input-search"
            placeholder="Filter (ULID, patient, responder)…"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        </div>
      </header>

      <section className="metric-strip">
        <MetricTile label="Open" value={metrics.open} tone="alert" />
        <MetricTile label="In handoff" value={metrics.handoff} tone="warn" />
        <MetricTile label="Closed (today)" value={metrics.closedToday} tone="success" />
        <MetricTile label="Total cases" value={metrics.total} />
      </section>

      <div className="active-grid">
        <section className="panel panel-table">
          <header className="panel-head">
            <h2>Cases</h2>
            <span className="muted">{filteredCases.length} shown</span>
          </header>
          <DataTable
            columns={columns}
            rows={filteredCases}
            rowKey={(r) => r.case_ulid}
            filterText={filter}
            filterMatch={(r, q) =>
              r.case_ulid.toLowerCase().includes(q) ||
              r.patient_label.toLowerCase().includes(q) ||
              r.opening_responder.toLowerCase().includes(q) ||
              r.case_type.toLowerCase().includes(q)
            }
            onRowClick={(r) => setSelected(r.case_ulid)}
            rowClassName={(r) => (selected === r.case_ulid ? "row-selected" : undefined)}
            empty="No active cases."
          />
        </section>

        <aside className="panel panel-map" aria-label="Map">
          <header className="panel-head">
            <h2>Map</h2>
            <span className="muted">v0.x — Leaflet/MapLibre</span>
          </header>
          <div className="map-placeholder">
            <div className="map-grid" aria-hidden />
            <div className="map-legend">
              <span className="dot dot-open" /> open
              <span className="dot dot-handoff" /> handoff
              <span className="dot dot-closed" /> closed
            </div>
            <p className="muted">
              Map placeholder. Will plot scene GPS for active cases.
            </p>
          </div>
        </aside>

        <aside className={`panel panel-drawer ${selected ? "open" : ""}`} aria-label="Case detail">
          <header className="panel-head">
            <h2>{drawerCase ? `Case ${shortUlid(drawerCase.case_ulid)}` : "Case detail"}</h2>
            {selected && (
              <button className="btn btn-ghost btn-sm" onClick={() => setSelected(null)}>
                close
              </button>
            )}
          </header>
          {!drawerCase && (
            <div className="empty">Select a case to view its timeline.</div>
          )}
          {drawerCase && <CaseDetail row={drawerCase} timeline={timeline} />}
        </aside>
      </div>

      {isMockMode && (
        <p className="footnote muted">
          MOCK MODE: cases sourced from <code>src/mock/cases.ts</code>. Set
          <code> VITE_USE_MOCK=0 </code> + a valid operator token to talk to a
          live storage server.
        </p>
      )}
    </div>
  );
}

function CaseDetail({ row, timeline }: { row: CaseRow; timeline: TimelineRow[] }) {
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  async function onForceClose() {
    if (!confirm(`Force-close case ${row.case_ulid}? This closes the operator's case grant; the patient retains a separate copy.`)) {
      return;
    }
    setBusy(true);
    setErr(null);
    const ok = await forceCloseCase(row.case_ulid, "operator force-close from dispatch console");
    setBusy(false);
    if (!ok) setErr("CloseCase failed; storage server log will have details.");
  }

  function onIssueReopenToken() {
    alert(
      `Issue reopen token for ${row.case_ulid}\n\n` +
        "TBD: storage does not yet expose an operator-side reopen-token issuance " +
        "RPC. Per SPEC §2.3 this lives on the relay; the dispatch UI will be the " +
        "issuer-side surface once the relay endpoint ships.",
    );
  }

  return (
    <div className="case-detail">
      <dl className="case-detail-meta">
        <div>
          <dt>ULID</dt>
          <dd className="mono">{row.case_ulid}</dd>
        </div>
        <div>
          <dt>Patient</dt>
          <dd>{row.patient_label}</dd>
        </div>
        <div>
          <dt>Status</dt>
          <dd><StatusChip status={row.status} /></dd>
        </div>
        <div>
          <dt>Opened</dt>
          <dd className="mono">{fmtStamp(row.opened_at_ms)}</dd>
        </div>
        <div>
          <dt>Elapsed</dt>
          <dd className="mono">{fmtElapsed(Date.now() - row.opened_at_ms)}</dd>
        </div>
        <div>
          <dt>Responder</dt>
          <dd>{row.opening_responder}</dd>
        </div>
        {row.destination && (
          <div>
            <dt>Destination</dt>
            <dd>{row.destination}</dd>
          </div>
        )}
      </dl>
      {row.scene_note && (
        <p className="case-detail-note">
          <span className="muted">Scene · </span>
          {row.scene_note}
        </p>
      )}

      <div className="case-detail-actions">
        <button
          type="button"
          className="btn btn-danger"
          onClick={onForceClose}
          disabled={busy || row.status === "closed"}
          title="Closes the operator's case grant. The patient's copy is unaffected."
        >
          Force close
        </button>
        <button
          type="button"
          className="btn"
          onClick={onIssueReopenToken}
          disabled={row.status !== "closed"}
          title="Issue a relay-signed reopen token (TBD)."
        >
          Issue reopen token
        </button>
      </div>
      {err && <p className="error-text">{err}</p>}

      <p className="case-detail-caveat muted">
        Per spec, only the patient can fully revoke a case. Force-close here
        closes the operator-side grant; the patient retains their own record.
      </p>

      <h3 className="section-title">Timeline</h3>
      <TimelineFeed rows={timeline} />
    </div>
  );
}
