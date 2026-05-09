import { useState } from "react";
import { useActivePatient } from "../PatientPage";
import { fmtDate } from "../../util";
import { NewEntryModal } from "../../components/NewEntryModal";
import { MOCK_OPERATOR } from "../../mock/store";
import type { EventType } from "../../types";

export function TimelineTab() {
  const patient = useActivePatient();
  const [open, setOpen] = useState(false);
  const [eventType, setEventType] = useState<EventType>("note");

  return (
    <section data-testid="tab-timeline">
      <div className="tab-header">
        <h2>Timeline</h2>
        <div style={{ display: "flex", gap: 8, alignItems: "center" }}>
          <select
            value={eventType}
            onChange={(e) => setEventType(e.target.value as EventType)}
            style={{
              padding: "6px 8px",
              border: "1px solid var(--color-border-strong)",
              borderRadius: 4,
              fontSize: 13,
            }}
            aria-label="New entry type"
          >
            <option value="note">Note</option>
            <option value="vital">Vital</option>
            <option value="symptom">Symptom</option>
            <option value="medication">Medication</option>
            <option value="lab">Lab</option>
            <option value="imaging">Imaging</option>
            <option value="food">Food</option>
          </select>
          <button className="btn btn-primary" onClick={() => setOpen(true)}>
            + New entry
          </button>
        </div>
      </div>

      {patient.timeline.length === 0 ? (
        <div className="empty">No events yet.</div>
      ) : (
        <ul className="timeline" data-testid="timeline-list">
          {patient.timeline.slice(0, 60).map((e, idx) => (
            <li key={idx} className="timeline-item">
              <span className="timeline-time">{fmtDate(e.ts_ms)}</span>
              <span className="timeline-type">{e.event_type}</span>
              <div>
                {e.summary}
                {e.detail && <div className="timeline-detail">{e.detail}</div>}
              </div>
            </li>
          ))}
        </ul>
      )}

      <NewEntryModal
        open={open}
        onClose={() => setOpen(false)}
        patientSlug={patient.slug}
        patientLabel={patient.label}
        approvalMode={patient.grant.approval_mode}
        eventType={eventType}
        operatorName={MOCK_OPERATOR.display_name}
      />
    </section>
  );
}
