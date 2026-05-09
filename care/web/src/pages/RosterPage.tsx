import { useNavigate } from "react-router-dom";
import { listPatients } from "../mock/store";
import { fmtRelative } from "../util";
import type { PatientSummary } from "../types";

/**
 * Roster — the multi-patient dashboard. Per SPEC §3.1:
 *   - Lists every patient in care_patient_grants where revocation_detected_ms IS NULL.
 *   - Status indicators: last-visit timestamp, recent flags, grant expiry warning.
 *   - Search by patient_label (deferred to v0.2 — small data, not painful yet).
 *   - "Add patient" → paste/scan share artifact (stub button for v0).
 *
 * v0 sources rows from src/mock/store; OHDC wiring lands in a later phase.
 */
export function RosterPage() {
  const navigate = useNavigate();
  const patients = listPatients();

  return (
    <div data-testid="roster-page">
      <div className="page-header">
        <div>
          <h1>Roster</h1>
          <p>
            {patients.length} patient{patients.length === 1 ? "" : "s"} have granted access.
            Click a card to open the patient view.
          </p>
        </div>
        <button
          className="btn"
          onClick={() => alert("Add-patient flow lands when grant-vault import is wired (paste/scan share artifact).")}
        >
          Add patient
        </button>
      </div>

      <div className="roster-grid">
        {patients.map((p) => (
          <RosterCard key={p.slug} p={p} onOpen={() => navigate(`/patient/${p.slug}`)} />
        ))}
      </div>
    </div>
  );
}

function RosterCard({ p, onOpen }: { p: PatientSummary; onOpen: () => void }) {
  return (
    <div
      className="roster-card"
      role="button"
      tabIndex={0}
      onClick={onOpen}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onOpen();
        }
      }}
      data-testid="roster-card"
    >
      <div className="roster-card-head">
        <div>
          <div className="roster-card-label">{p.label}</div>
          <div className="roster-card-display">{p.display_name}</div>
        </div>
        {p.active_case && <span className="flag flag-active">case open</span>}
        {p.grant.status === "case_bound" && !p.active_case && (
          <span className="flag flag-info">curated visit</span>
        )}
        {p.grant.status === "expiring_soon" && <span className="flag">grant expiring</span>}
      </div>

      <div className="roster-card-meta">
        <span>Last visit: {fmtRelative(p.last_visit_ms)}</span>
        <span>· Approval: {p.grant.approval_mode.replace(/_/g, " ")}</span>
      </div>

      {p.flags.length > 0 && (
        <div className="flag-row">
          {p.flags.map((f, idx) => (
            <span key={idx} className="flag">
              {f}
            </span>
          ))}
        </div>
      )}

      <ul className="roster-card-meds">
        {p.meds_summary.slice(0, 3).map((m, idx) => (
          <li key={idx}>{m}</li>
        ))}
      </ul>
    </div>
  );
}
