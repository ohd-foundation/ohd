import { useState } from "react";
import { Column, DataTable } from "../components/DataTable";
import { getOperatorRecords } from "../mock/store";
import type { OperatorRecordRow } from "../types";
import { downloadText, fmtStamp, shortUlid, toCsv } from "../util";

export function OperatorRecordsPage() {
  const rows = getOperatorRecords();
  const [filter, setFilter] = useState("");

  function exportCsv() {
    const csv = toCsv(
      ["case_ulid", "patient_label", "opened_at", "closed_at", "authority", "destination", "billing", "auto_granted"],
      rows.map((r) => [
        r.case_ulid,
        r.patient_label,
        fmtStamp(r.opened_at_ms),
        fmtStamp(r.closed_at_ms),
        r.authority_label,
        r.destination,
        r.billing_status,
        String(r.auto_granted),
      ]),
    );
    downloadText(`ohd-dispatch-records-${Date.now()}.csv`, csv);
  }

  const columns: Column<OperatorRecordRow>[] = [
    {
      key: "case_ulid",
      header: "Case",
      width: "150px",
      cell: (r) => <span className="mono">{shortUlid(r.case_ulid)}</span>,
    },
    {
      key: "patient_label",
      header: "Patient",
      width: "120px",
      cell: (r) => <strong>{r.patient_label}</strong>,
    },
    {
      key: "opened_at",
      header: "Opened",
      width: "180px",
      cell: (r) => <span className="mono">{fmtStamp(r.opened_at_ms)}</span>,
      sort: (a, b) => a.opened_at_ms - b.opened_at_ms,
      align: "num",
    },
    {
      key: "closed_at",
      header: "Closed",
      width: "180px",
      cell: (r) => <span className="mono">{fmtStamp(r.closed_at_ms)}</span>,
      sort: (a, b) => a.closed_at_ms - b.closed_at_ms,
      align: "num",
    },
    { key: "destination", header: "Destination" },
    { key: "billing_status", header: "Billing", width: "120px" },
    {
      key: "auto_granted",
      header: "Auto-granted",
      width: "120px",
      cell: (r) => (r.auto_granted ? "yes" : "no"),
    },
  ];

  return (
    <div className="page" data-testid="operator-records-page">
      <header className="page-head">
        <div>
          <h1>Operator records</h1>
          <p className="muted">
            Local DB of completed case archives — separate from patient OHD.
          </p>
        </div>
        <div className="page-head-actions">
          <input
            type="search"
            className="input input-search"
            placeholder="Filter (ULID, patient, destination)…"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
          <button type="button" className="btn" onClick={exportCsv}>
            Export CSV
          </button>
        </div>
      </header>

      <div className="banner banner-info">
        <strong>v0 placeholder.</strong> Per SPEC §5 the operator records DB is
        a Postgres schema living outside OHDC entirely (subject to the
        operator's retention policy / regulatory regime). The reference
        deployment's <code>postgres-records</code> service is in
        <code> ../deploy/docker-compose.yml</code>; the schema + connector
        ship in v0.x.
      </div>

      <section className="panel">
        <DataTable
          columns={columns}
          rows={rows}
          rowKey={(r) => r.case_ulid}
          filterText={filter}
          filterMatch={(r, q) =>
            r.case_ulid.toLowerCase().includes(q) ||
            r.patient_label.toLowerCase().includes(q) ||
            r.destination.toLowerCase().includes(q)
          }
        />
      </section>
    </div>
  );
}
