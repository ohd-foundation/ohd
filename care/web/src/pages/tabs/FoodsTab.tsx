import { useState } from "react";
import { useActivePatient } from "../PatientPage";
import { NewEntryModal } from "../../components/NewEntryModal";
import { MOCK_OPERATOR } from "../../mock/store";
import { fmtDate } from "../../util";

export function FoodsTab() {
  const patient = useActivePatient();
  const [open, setOpen] = useState(false);

  return (
    <section data-testid="tab-foods">
      <div className="tab-header">
        <h2>Foods</h2>
        <button className="btn btn-primary" onClick={() => setOpen(true)}>
          + Log food
        </button>
      </div>

      {patient.foods.length === 0 ? (
        <div className="empty">No food entries recorded.</div>
      ) : (
        <table className="data-table">
          <thead>
            <tr>
              <th>When</th>
              <th>Entry</th>
              <th className="num">~kcal</th>
            </tr>
          </thead>
          <tbody>
            {patient.foods.map((f, idx) => (
              <tr key={idx}>
                <td>{fmtDate(f.ts_ms)}</td>
                <td>{f.text}</td>
                <td className="num">{f.kcal ?? "—"}</td>
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
        eventType="food"
        operatorName={MOCK_OPERATOR.display_name}
      />
    </section>
  );
}
