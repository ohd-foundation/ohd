import { useState } from "react";
import { useActivePatient } from "../PatientPage";
import { NewEntryModal } from "../../components/NewEntryModal";
import { MOCK_OPERATOR } from "../../mock/store";
import { fmtRelative } from "../../util";

export function MedicationsTab() {
  const patient = useActivePatient();
  const [open, setOpen] = useState(false);

  return (
    <section data-testid="tab-medications">
      <div className="tab-header">
        <h2>Medications</h2>
        <button className="btn btn-primary" onClick={() => setOpen(true)}>
          + Prescribe
        </button>
      </div>

      {patient.medications.length === 0 ? (
        <div className="empty">No medications recorded for this patient.</div>
      ) : (
        patient.medications.map((m, idx) => {
          const taken = m.recent_doses.filter((d) => d.taken).length;
          const total = m.recent_doses.length;
          const adherence = total > 0 ? Math.round((taken / total) * 100) : null;
          const last = m.recent_doses[m.recent_doses.length - 1];
          return (
            <div key={idx} className="card">
              <div className="card-title">
                <h3>
                  {m.name}{" "}
                  <span className="muted mono" style={{ fontWeight: 400, fontSize: 13 }}>
                    {m.dose}
                  </span>
                </h3>
                <span className="muted" style={{ fontSize: 12 }}>
                  {m.active ? "active" : "discontinued"}
                </span>
              </div>
              <div style={{ display: "flex", gap: 24, fontSize: 13 }}>
                <div>
                  <h4 style={{ marginBottom: 4 }}>Schedule</h4>
                  <div>{m.schedule}</div>
                </div>
                {adherence != null && (
                  <div>
                    <h4 style={{ marginBottom: 4 }}>Adherence</h4>
                    <div className="mono">
                      {adherence}% ({taken}/{total} of recent doses)
                    </div>
                  </div>
                )}
                {last && (
                  <div>
                    <h4 style={{ marginBottom: 4 }}>Last logged</h4>
                    <div>
                      {fmtRelative(last.ts_ms)} {last.taken ? "(taken)" : "(missed)"}
                    </div>
                  </div>
                )}
              </div>
            </div>
          );
        })
      )}

      <NewEntryModal
        open={open}
        onClose={() => setOpen(false)}
        patientSlug={patient.slug}
        patientLabel={patient.label}
        approvalMode={patient.grant.approval_mode}
        eventType="medication"
        operatorName={MOCK_OPERATOR.display_name}
      />
    </section>
  );
}
