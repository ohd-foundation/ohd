import { Link, NavLink, Outlet, useOutletContext, useParams } from "react-router-dom";
import { getPatientBySlug } from "../mock/store";
import { fmtApprovalMode, fmtDate, fmtRelative, fmtScope } from "../util";
import type { PatientDetail } from "../types";

const TABS: { path: string; label: string }[] = [
  { path: "timeline", label: "Timeline" },
  { path: "vitals", label: "Vitals" },
  { path: "medications", label: "Medications" },
  { path: "symptoms", label: "Symptoms" },
  { path: "foods", label: "Foods" },
  { path: "labs", label: "Labs" },
  { path: "imaging", label: "Imaging" },
  { path: "notes", label: "Notes" },
];

export interface PatientTabContext {
  patient: PatientDetail;
}

/**
 * Per-patient view. Header (active patient + grant scope + active case) is the
 * §3.3 multi-patient safety property; visit-prep brief sits below the header;
 * tabs render the per-channel detail and the per-tab "New entry" button opens
 * the submission modal.
 */
export function PatientPage() {
  const { label } = useParams<{ label: string }>();
  const patient = label ? getPatientBySlug(label) : undefined;

  if (!patient) {
    return (
      <div className="empty">
        <p>
          No patient with slug <code>{label}</code> in the mock store.
        </p>
        <Link to="/roster">Back to roster</Link>
      </div>
    );
  }

  return (
    <div>
      <PatientHeader patient={patient} />
      <Brief patient={patient} />
      <nav className="tabs" aria-label="Patient sections">
        {TABS.map((t) => (
          <NavLink
            key={t.path}
            to={`/patient/${patient.slug}/${t.path}`}
            className={({ isActive }) => (isActive ? "active" : "")}
          >
            {t.label}
          </NavLink>
        ))}
      </nav>
      <Outlet context={{ patient } satisfies PatientTabContext} />
    </div>
  );
}

/** Re-exposed hook so per-tab components don't need to re-fetch the patient. */
export function useActivePatient(): PatientDetail {
  return useOutletContext<PatientTabContext>().patient;
}

function PatientHeader({ patient }: { patient: PatientDetail }) {
  return (
    <header className="patient-header">
      <div className="patient-label-bar">
        <div className="label">Active patient</div>
        <h1>{patient.label}</h1>
        <div className="muted" style={{ fontSize: 12, marginTop: 2 }}>
          {patient.display_name} · last visit {fmtRelative(patient.last_visit_ms)}
        </div>
      </div>

      <dl className="patient-grant-row">
        <div>
          <dt>Read scope</dt>
          <dd>{fmtScope(patient.grant.read_scope)}</dd>
        </div>
        <div>
          <dt>Write scope</dt>
          <dd>{fmtScope(patient.grant.write_scope)}</dd>
        </div>
        <div>
          <dt>Approval mode</dt>
          <dd>{fmtApprovalMode(patient.grant.approval_mode)}</dd>
        </div>
        <div>
          <dt>Grant expiry</dt>
          <dd>
            {patient.grant.expires_at_ms
              ? `${fmtRelative(patient.grant.expires_at_ms)} (${fmtDate(patient.grant.expires_at_ms)})`
              : "open-ended"}
          </dd>
        </div>
      </dl>

      {patient.grant.status === "case_bound" && patient.grant.case_label && !patient.active_case && (
        <div className="case-banner">
          <span>
            <strong>Curated visit</strong> — {patient.grant.case_label}
            {patient.grant.case_event_count != null && ` · ${patient.grant.case_event_count} events linked`}
          </span>
          <span className="mono muted" style={{ fontSize: 12 }}>
            read scope filtered by patient
          </span>
        </div>
      )}

      {patient.active_case && (
        <div className="case-banner" data-testid="active-case-banner">
          <span>
            <strong>{patient.active_case.authority}</strong> — {patient.active_case.label}; open since{" "}
            {fmtDate(patient.active_case.started_ms)}
          </span>
          <span className="mono muted" style={{ fontSize: 12 }}>
            predecessor case visible
          </span>
        </div>
      )}
    </header>
  );
}

function Brief({ patient }: { patient: PatientDetail }) {
  if (patient.brief.length === 0) return null;
  return (
    <section className="brief" aria-label="Visit-prep brief">
      <h4>Visit-prep brief</h4>
      <ul>
        {patient.brief.map((b, idx) => (
          <li key={idx}>{b}</li>
        ))}
      </ul>
    </section>
  );
}
