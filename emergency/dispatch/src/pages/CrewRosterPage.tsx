import { useState } from "react";
import { Column, DataTable } from "../components/DataTable";
import { MetricTile } from "../components/MetricTile";
import { StatusChip } from "../components/StatusChip";
import { getCrew } from "../mock/store";
import type { CrewMember } from "../types";
import { fmtElapsed } from "../util";

export function CrewRosterPage() {
  const crew = getCrew();
  const [filter, setFilter] = useState("");

  const onDuty = crew.filter((c) => c.on_duty).length;
  const inCase = crew.filter((c) => c.current_case_ulid != null).length;
  const available = onDuty - inCase;

  const columns: Column<CrewMember>[] = [
    {
      key: "display_name",
      header: "Paramedic",
      width: "180px",
      cell: (r) => (
        <div>
          <strong>{r.display_name}</strong>
          <div className="muted mono">{r.responder_label}</div>
        </div>
      ),
      sort: (a, b) => a.display_name.localeCompare(b.display_name),
    },
    {
      key: "on_duty",
      header: "Duty",
      width: "100px",
      cell: (r) => <StatusChip status={r.on_duty ? "open" : "closed"} />,
      sort: (a, b) => Number(a.on_duty) - Number(b.on_duty),
    },
    {
      key: "since",
      header: "On duty since",
      width: "150px",
      cell: (r) =>
        r.on_duty_since_ms
          ? <span className="mono">{fmtElapsed(Date.now() - r.on_duty_since_ms)} ago</span>
          : <span className="muted">—</span>,
      sort: (a, b) =>
        (a.on_duty_since_ms ?? 0) - (b.on_duty_since_ms ?? 0),
      align: "num",
    },
    {
      key: "current_case",
      header: "Current case",
      cell: (r) =>
        r.current_case_ulid
          ? <span className="mono">{r.current_case_ulid.slice(0, 10)}…</span>
          : <span className="muted">available</span>,
    },
    {
      key: "last_seen",
      header: "Last seen",
      width: "120px",
      cell: (r) => <span className="mono">{fmtElapsed(Date.now() - r.last_seen_ms)} ago</span>,
      sort: (a, b) => a.last_seen_ms - b.last_seen_ms,
      align: "num",
    },
    {
      key: "contact",
      header: "Contact",
      width: "160px",
      cell: (r) => <span className="mono">{r.contact}</span>,
    },
    {
      key: "actions",
      header: "",
      width: "180px",
      cell: (r) => (
        <div className="row-actions">
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => alert(`TODO: toggle duty for ${r.display_name} (operator-side state; v0 stub).`)}
          >
            {r.on_duty ? "Mark off-duty" : "Mark on-duty"}
          </button>
        </div>
      ),
    },
  ];

  return (
    <div className="page" data-testid="crew-roster-page">
      <header className="page-head">
        <div>
          <h1>Crew roster</h1>
          <p className="muted">Operator-side state. Authorize, mark on/off duty, contact.</p>
        </div>
        <div className="page-head-actions">
          <input
            type="search"
            className="input input-search"
            placeholder="Filter…"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
          <button
            type="button"
            className="btn btn-primary"
            onClick={() => alert("TODO: authorize new responder (operator-side; v0 stub).")}
          >
            Authorize responder
          </button>
        </div>
      </header>

      <section className="metric-strip">
        <MetricTile label="On duty" value={onDuty} tone="success" />
        <MetricTile label="In a case" value={inCase} tone="alert" />
        <MetricTile label="Available" value={available} tone="warn" />
      </section>

      <section className="panel">
        <DataTable
          columns={columns}
          rows={crew}
          rowKey={(r) => r.responder_label}
          filterText={filter}
          filterMatch={(r, q) =>
            r.display_name.toLowerCase().includes(q) ||
            r.responder_label.toLowerCase().includes(q) ||
            (r.current_case_ulid?.toLowerCase().includes(q) ?? false)
          }
        />
      </section>

      <p className="footnote muted">
        Roster is operator-side state — sourced from the operator IdP via the
        relay's roster sync. v0 ships mock data; relay wiring lands in v0.x.
      </p>
    </div>
  );
}
