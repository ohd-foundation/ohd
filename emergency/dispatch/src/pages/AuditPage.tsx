import { useMemo, useState } from "react";
import { Column, DataTable } from "../components/DataTable";
import { ResultChip } from "../components/StatusChip";
import { getAudit } from "../mock/store";
import type { AuditRow } from "../types";
import { downloadText, fmtStamp, shortUlid, toCsv } from "../util";

export function AuditPage() {
  const audit = getAudit();
  const [filter, setFilter] = useState("");
  const [responder, setResponder] = useState<string>("all");
  const [result, setResult] = useState<string>("all");

  const filtered = useMemo(() => {
    return audit.filter((r) => {
      if (responder !== "all" && r.responder_label !== responder) return false;
      if (result !== "all" && r.result !== result) return false;
      return true;
    });
  }, [audit, responder, result]);

  const responders = useMemo(
    () => Array.from(new Set(audit.map((r) => r.responder_label))).sort(),
    [audit],
  );

  function exportCsv() {
    const rows = filtered.map((r) => [
      fmtStamp(r.ts_ms),
      r.responder_label,
      r.case_ulid,
      r.action,
      r.result,
      r.scope,
      r.caller_ip ?? "",
    ]);
    const csv = toCsv(
      ["timestamp", "responder", "case_ulid", "action", "result", "scope", "caller_ip"],
      rows,
    );
    downloadText(`ohd-dispatch-audit-${Date.now()}.csv`, csv);
  }

  const columns: Column<AuditRow>[] = [
    {
      key: "ts_ms",
      header: "Timestamp (UTC)",
      width: "190px",
      cell: (r) => <span className="mono">{fmtStamp(r.ts_ms)}</span>,
      sort: (a, b) => a.ts_ms - b.ts_ms,
      align: "num",
    },
    {
      key: "responder_label",
      header: "Responder",
      width: "140px",
      cell: (r) => <span className="mono">{r.responder_label}</span>,
      sort: (a, b) => a.responder_label.localeCompare(b.responder_label),
    },
    {
      key: "case_ulid",
      header: "Case",
      width: "150px",
      cell: (r) => <span className="mono">{shortUlid(r.case_ulid)}</span>,
      sort: (a, b) => a.case_ulid.localeCompare(b.case_ulid),
    },
    {
      key: "action",
      header: "Action",
      width: "180px",
      sort: (a, b) => a.action.localeCompare(b.action),
    },
    {
      key: "result",
      header: "Result",
      width: "100px",
      cell: (r) => <ResultChip result={r.result} />,
      sort: (a, b) => a.result.localeCompare(b.result),
    },
    {
      key: "scope",
      header: "Scope / detail",
      cell: (r) => <span className="muted">{r.scope}</span>,
    },
    {
      key: "caller_ip",
      header: "IP",
      width: "120px",
      cell: (r) => <span className="mono muted">{r.caller_ip ?? "—"}</span>,
    },
  ];

  return (
    <div className="page" data-testid="audit-page">
      <header className="page-head">
        <div>
          <h1>Audit</h1>
          <p className="muted">Break-glass initiations + recent operator-side actions.</p>
        </div>
        <div className="page-head-actions">
          <select
            className="input"
            value={responder}
            onChange={(e) => setResponder(e.target.value)}
            aria-label="Filter by responder"
          >
            <option value="all">All responders</option>
            {responders.map((r) => (
              <option key={r} value={r}>{r}</option>
            ))}
          </select>
          <select
            className="input"
            value={result}
            onChange={(e) => setResult(e.target.value)}
            aria-label="Filter by result"
          >
            <option value="all">All results</option>
            <option value="success">Success</option>
            <option value="partial">Partial</option>
            <option value="rejected">Rejected</option>
            <option value="error">Error</option>
          </select>
          <input
            type="search"
            className="input input-search"
            placeholder="Filter (action, scope)…"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
          <button type="button" className="btn" onClick={exportCsv}>
            Export CSV
          </button>
        </div>
      </header>

      <div className="banner banner-info">
        <strong>TBD:</strong> storage <code>OhdcService.AuditQuery</code> RPC is
        currently stubbed (returns NOT_IMPLEMENTED — see
        <code> ../../storage/STATUS.md</code>). The rows below are mock data so
        the page layout is reviewable now. Wiring to the real RPC is a single
        store change once it ships.
      </div>

      <section className="panel">
        <DataTable
          columns={columns}
          rows={filtered}
          rowKey={(r) => `${r.ts_ms}-${r.responder_label}-${r.action}`}
          filterText={filter}
          filterMatch={(r, q) =>
            r.action.toLowerCase().includes(q) ||
            r.scope.toLowerCase().includes(q) ||
            r.case_ulid.toLowerCase().includes(q)
          }
          empty="No audit rows match the current filters."
        />
      </section>
    </div>
  );
}
