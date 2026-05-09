import { Link, Outlet, useMatch } from "react-router-dom";
import { MOCK_OPERATOR, getPatientBySlug } from "../mock/store";
import { forgetGrantToken } from "../ohdc/client";
import { clearSession, loadSession } from "../ohdc/oidc";

/**
 * Top-level app frame. Always renders the operator + active-patient bar so
 * SPEC §3.3 "active patient prominently displayed everywhere" holds across
 * route changes.
 *
 * The active-patient lookup happens via a route match against /patient/:label
 * — when there is no active patient, the slot reads "No patient active".
 *
 * The operator's display comes from the OIDC session in sessionStorage
 * (set by OidcCallbackPage). When no session is present we fall back to
 * the mock operator so the demo flow without a clinic SSO is still
 * navigable.
 */
export function AppShell() {
  const patientMatch = useMatch("/patient/:label/*");
  const slug = patientMatch?.params.label;
  const patient = slug ? getPatientBySlug(slug) : undefined;

  const session = loadSession();
  const operatorName = session?.displayName ?? session?.email ?? MOCK_OPERATOR.display_name;
  const operatorTag = session ? "OIDC" : MOCK_OPERATOR.role;

  return (
    <div className="app-shell">
      <header className="topbar">
        <Link to="/roster" className="topbar-brand" aria-label="OHD Care home">
          <span className="topbar-brand-name">OHD Care</span>
          <span className="topbar-brand-tag">v0</span>
        </Link>
        <nav className="topbar-nav" aria-label="primary">
          <Link to="/roster" className="topbar-nav-link">Roster</Link>
          <Link to="/pending" className="topbar-nav-link">Pending</Link>
          <Link to="/chat" className="topbar-nav-link">Chat</Link>
          <Link to="/audit" className="topbar-nav-link">Audit</Link>
          <Link to="/settings/mcp" className="topbar-nav-link">Settings</Link>
        </nav>
        <div className="topbar-active-patient" aria-live="polite">
          {patient ? (
            <>
              <span className="muted">Active patient:</span>
              <strong>{patient.label}</strong>
              {patient.active_case && (
                <span className="flag flag-active">case open</span>
              )}
            </>
          ) : (
            <span className="muted">No patient active</span>
          )}
        </div>
        <div className="topbar-operator">
          <span className={`status-dot ${session ? "" : "idle"}`} />
          <span>{operatorName}</span>
          <span className="muted">· {operatorTag}</span>
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            onClick={() => {
              // Clear both the operator OIDC session and the per-patient
              // grant. Local-side only — server-side session revocation
              // (`/auth/logout`) is the storage-roadmap deliverable.
              clearSession();
              forgetGrantToken();
              window.location.href = "/login";
            }}
          >
            Sign out
          </button>
        </div>
      </header>
      <main className="main">
        <Outlet />
      </main>
    </div>
  );
}
