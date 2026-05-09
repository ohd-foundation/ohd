import { useMemo, useState } from "react";
import { useActivePatient } from "../PatientPage";
import { fmtDate } from "../../util";
import { NewEntryModal } from "../../components/NewEntryModal";
import { MOCK_OPERATOR } from "../../mock/store";

export function VitalsTab() {
  const patient = useActivePatient();
  const [open, setOpen] = useState(false);

  const channels = useMemo(() => {
    const grouped = new Map<string, { ts_ms: number; value: number; unit: string }[]>();
    for (const v of patient.vitals) {
      const list = grouped.get(v.channel) ?? [];
      list.push({ ts_ms: v.ts_ms, value: v.value, unit: v.unit });
      grouped.set(v.channel, list);
    }
    for (const list of grouped.values()) list.sort((a, b) => a.ts_ms - b.ts_ms);
    return [...grouped.entries()];
  }, [patient.vitals]);

  return (
    <section data-testid="tab-vitals">
      <div className="tab-header">
        <h2>Vitals</h2>
        <button className="btn btn-primary" onClick={() => setOpen(true)}>
          + New vital
        </button>
      </div>

      {channels.length === 0 ? (
        <div className="empty">No vitals recorded for this patient.</div>
      ) : (
        channels.map(([channel, points]) => (
          <div key={channel} className="card">
            <div className="card-title">
              <h3>{prettyChannel(channel)}</h3>
              <span className="muted mono" style={{ fontSize: 12 }}>
                {points.length} reading{points.length === 1 ? "" : "s"} · last{" "}
                {fmtDate(points[points.length - 1].ts_ms)}
              </span>
            </div>
            <Sparkline points={points.map((p) => p.value)} />
            <table className="data-table" style={{ marginTop: 10 }}>
              <thead>
                <tr>
                  <th>When</th>
                  <th className="num">Value</th>
                  <th>Unit</th>
                </tr>
              </thead>
              <tbody>
                {points
                  .slice(-5)
                  .reverse()
                  .map((p, idx) => (
                    <tr key={idx}>
                      <td>{fmtDate(p.ts_ms)}</td>
                      <td className="num">{p.value.toFixed(1)}</td>
                      <td>{p.unit}</td>
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
        eventType="vital"
        operatorName={MOCK_OPERATOR.display_name}
      />
    </section>
  );
}

function prettyChannel(c: string): string {
  switch (c) {
    case "bp_systolic":
      return "Blood pressure (systolic)";
    case "bp_diastolic":
      return "Blood pressure (diastolic)";
    case "hr":
      return "Heart rate";
    case "temp_c":
      return "Temperature (°C)";
    case "spo2":
      return "SpO2";
    case "glucose_mg_dl":
      return "Glucose";
    default:
      return c;
  }
}

/**
 * Inline SVG sparkline — no chart library dependency. Real charts arrive when
 * the visit-prep panel binds to OHDC's Auth.Chart.
 */
function Sparkline({ points }: { points: number[] }) {
  if (points.length === 0) return null;
  const w = 600;
  const h = 60;
  const min = Math.min(...points);
  const max = Math.max(...points);
  const span = max - min || 1;
  const dx = w / Math.max(points.length - 1, 1);
  const path = points
    .map((v, i) => `${i === 0 ? "M" : "L"} ${(i * dx).toFixed(1)} ${(h - ((v - min) / span) * (h - 8) - 4).toFixed(1)}`)
    .join(" ");
  return (
    <svg viewBox={`0 0 ${w} ${h}`} className="sparkline" preserveAspectRatio="none" aria-hidden="true">
      <path d={path} fill="none" stroke="var(--color-accent)" strokeWidth="1.5" />
    </svg>
  );
}
