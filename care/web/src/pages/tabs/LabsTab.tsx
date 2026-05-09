import { useState } from "react";
import { useActivePatient } from "../PatientPage";
import { NewEntryModal } from "../../components/NewEntryModal";
import { MOCK_OPERATOR } from "../../mock/store";
import { fmtDate } from "../../util";

export function LabsTab() {
  const patient = useActivePatient();
  const [open, setOpen] = useState(false);

  return (
    <section data-testid="tab-labs">
      <div className="tab-header">
        <h2>Labs</h2>
        <button className="btn btn-primary" onClick={() => setOpen(true)}>
          + Submit lab
        </button>
      </div>

      {patient.labs.length === 0 ? (
        <div className="empty">No lab panels recorded.</div>
      ) : (
        patient.labs.map((lab, idx) => (
          <div key={idx} className="card">
            <div className="card-title">
              <h3>{lab.panel}</h3>
              <span className="muted mono" style={{ fontSize: 12 }}>
                {fmtDate(lab.ts_ms)}
              </span>
            </div>
            <table className="data-table">
              <thead>
                <tr>
                  <th>Analyte</th>
                  <th>Value</th>
                  <th>Reference</th>
                </tr>
              </thead>
              <tbody>
                {lab.values.map((v, j) => (
                  <tr key={j}>
                    <td>{v.name}</td>
                    <td>
                      <span className="mono">{v.value}</span>
                      {v.flag && v.flag !== "normal" && (
                        <span className={`value-flag ${v.flag}`}>{v.flag}</span>
                      )}
                    </td>
                    <td className="muted">{v.range ?? "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ))
      )}

      <NewEntryModal
        open={open}
        onClose={() => setOpen(false)}
        patientSlug={patient.slug}
        patientLabel={patient.label}
        approvalMode={patient.grant.approval_mode}
        eventType="lab"
        operatorName={MOCK_OPERATOR.display_name}
      />
    </section>
  );
}
