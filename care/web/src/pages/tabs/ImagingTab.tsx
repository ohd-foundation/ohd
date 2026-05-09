import { useState } from "react";
import { useActivePatient } from "../PatientPage";
import { NewEntryModal } from "../../components/NewEntryModal";
import { MOCK_OPERATOR } from "../../mock/store";
import { fmtDate } from "../../util";

export function ImagingTab() {
  const patient = useActivePatient();
  const [open, setOpen] = useState(false);

  return (
    <section data-testid="tab-imaging">
      <div className="tab-header">
        <h2>Imaging</h2>
        <button className="btn btn-primary" onClick={() => setOpen(true)}>
          + Submit finding
        </button>
      </div>

      {patient.imaging.length === 0 ? (
        <div className="empty">No imaging studies recorded.</div>
      ) : (
        patient.imaging.map((s, idx) => (
          <div key={idx} className="card">
            <div className="card-title">
              <h3>
                {s.modality} · {s.region}
              </h3>
              <span className="muted mono" style={{ fontSize: 12 }}>
                {fmtDate(s.ts_ms)}
              </span>
            </div>
            <p style={{ margin: 0 }}>{s.findings}</p>
          </div>
        ))
      )}

      <NewEntryModal
        open={open}
        onClose={() => setOpen(false)}
        patientSlug={patient.slug}
        patientLabel={patient.label}
        approvalMode={patient.grant.approval_mode}
        eventType="imaging"
        operatorName={MOCK_OPERATOR.display_name}
      />
    </section>
  );
}
