import { useState } from "react";
import { useActivePatient } from "../PatientPage";
import { NewEntryModal } from "../../components/NewEntryModal";
import { MOCK_OPERATOR } from "../../mock/store";
import { fmtDate } from "../../util";

export function NotesTab() {
  const patient = useActivePatient();
  const [open, setOpen] = useState(false);

  return (
    <section data-testid="tab-notes">
      <div className="tab-header">
        <h2>Notes</h2>
        <button className="btn btn-primary" onClick={() => setOpen(true)}>
          + New note
        </button>
      </div>

      {patient.notes.length === 0 ? (
        <div className="empty">No clinical notes yet.</div>
      ) : (
        patient.notes.map((n, idx) => (
          <div key={idx} className="card">
            <div className="card-title">
              <h3>
                {n.author}{" "}
                <span className="muted" style={{ fontWeight: 400, fontSize: 12 }}>
                  · {fmtDate(n.ts_ms)}
                </span>
              </h3>
              <span className={`flag ${statusClass(n.status)}`}>{n.status.replace(/_/g, " ")}</span>
            </div>
            <p style={{ margin: 0, whiteSpace: "pre-wrap" }}>{n.text}</p>
          </div>
        ))
      )}

      <NewEntryModal
        open={open}
        onClose={() => setOpen(false)}
        patientSlug={patient.slug}
        patientLabel={patient.label}
        approvalMode={patient.grant.approval_mode}
        eventType="note"
        operatorName={MOCK_OPERATOR.display_name}
      />
    </section>
  );
}

function statusClass(status: string): string {
  if (status === "pending_patient_approval") return "";
  if (status === "auto_committed") return "flag-info";
  return "flag-success";
}
