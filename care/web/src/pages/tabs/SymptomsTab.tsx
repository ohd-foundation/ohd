import { useState } from "react";
import { useActivePatient } from "../PatientPage";
import { NewEntryModal } from "../../components/NewEntryModal";
import { MOCK_OPERATOR } from "../../mock/store";
import { fmtDate } from "../../util";

export function SymptomsTab() {
  const patient = useActivePatient();
  const [open, setOpen] = useState(false);

  return (
    <section data-testid="tab-symptoms">
      <div className="tab-header">
        <h2>Symptoms</h2>
        <button className="btn btn-primary" onClick={() => setOpen(true)}>
          + Log symptom
        </button>
      </div>

      {patient.symptoms.length === 0 ? (
        <div className="empty">No symptoms recorded.</div>
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th>When</th>
              <th>Description</th>
              <th className="num">Severity</th>
            </tr>
          </thead>
          <tbody>
            {patient.symptoms.map((s, idx) => (
              <tr key={idx}>
                <td>{fmtDate(s.ts_ms)}</td>
                <td>{s.text}</td>
                <td className="num">{s.severity} / 5</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      <NewEntryModal
        open={open}
        onClose={() => setOpen(false)}
        patientSlug={patient.slug}
        patientLabel={patient.label}
        approvalMode={patient.grant.approval_mode}
        eventType="symptom"
        operatorName={MOCK_OPERATOR.display_name}
      />
    </section>
  );
}
