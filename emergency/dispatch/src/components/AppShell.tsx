import { NavLink, Outlet, useNavigate } from "react-router-dom";
import { fmtClock } from "../util";
import { getSession, isMockMode } from "../mock/store";
import { useStoreVersion } from "../ohdc/useStore";
import { clearSession, loadSession } from "../ohdc/oidc";
import { forgetOperatorToken } from "../ohdc/client";

/**
 * Dispatch console frame.
 *
 * Layout: top bar (station + operator + clock + status) + left sidebar
 * (5 nav items) + main content area. Dense by default — desktop monitor
 * (often multi-monitor), unlike the tablet which is mobile-first.
 */
export function AppShell() {
  // Re-render on every snapshot bump so the cases-count badge stays fresh.
  useStoreVersion();
  const navigate = useNavigate();
  const session = getSession();
  // Operator OIDC session (if signed in via /login). Falls back to the
  // mock-store operator name otherwise.
  const oidcSession = loadSession();
  const operatorDisplayName =
    oidcSession?.operatorName ?? session.operator_display_name;
  const now = Date.now();
  const certExpiresHrs = Math.max(
    0,
    Math.round((session.authority_cert_expires_at_ms - now) / 3600_000),
  );

  function signOut() {
    clearSession();
    forgetOperatorToken();
    navigate("/login", { replace: true });
  }

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="topbar-brand">
          <span className="brand-mark">OHD</span>
          <span className="brand-tag">EMERGENCY · DISPATCH</span>
        </div>
        <div className="topbar-station">
          <div className="topbar-station-label">{session.station_label}</div>
          <div className="topbar-station-meta">
            authority cert expires in {certExpiresHrs}h
            {isMockMode ? " · MOCK MODE" : ""}
          </div>
        </div>
        <div className="topbar-clock" aria-live="polite">
          <span className="clock-time">{fmtClock(now)}</span>
          <span className="clock-label">UTC</span>
        </div>
        <div className="topbar-operator">
          <span className="status-dot status-dot-online" aria-hidden />
          <span className="operator-name">{operatorDisplayName}</span>
          <span className="operator-role">dispatcher</span>
          {oidcSession && (
            <button
              type="button"
              className="btn btn-ghost btn-small"
              onClick={signOut}
              title="Sign out and clear operator OIDC session"
              style={{ marginLeft: 8 }}
            >
              Sign out
            </button>
          )}
        </div>
      </header>
      <div className="shell-body">
        <nav className="sidebar" aria-label="Dispatch sections">
          <SidebarItem to="/active" label="Active cases" hint="live" />
          <SidebarItem to="/roster" label="Crew roster" />
          <SidebarItem to="/audit" label="Audit" />
          <SidebarItem to="/records" label="Operator records" />
          <SidebarItem to="/settings" label="Settings" />
          <div className="sidebar-foot">
            <span className="muted">v0 · spec/SPEC.md</span>
          </div>
        </nav>
        <main className="main">
          <Outlet />
        </main>
      </div>
    </div>
  );
}

function SidebarItem({ to, label, hint }: { to: string; label: string; hint?: string }) {
  return (
    <NavLink
      to={to}
      className={({ isActive }) => `sidebar-item ${isActive ? "active" : ""}`}
    >
      <span className="sidebar-item-label">{label}</span>
      {hint && <span className="sidebar-item-hint">{hint}</span>}
    </NavLink>
  );
}
