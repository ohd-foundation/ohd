import { NavLink, Outlet } from "react-router-dom";

/**
 * Settings shell. Sub-routes:
 *   - /settings/storage    — URL, token, deployment switch
 *   - /settings/emergency  — break-glass feature config (the meat of this page)
 *   - /settings/cases      — active + closed cases, force-close, retro grants
 *   - /settings/delegates  — family / delegate access (act-on-behalf grants)
 *   - /settings/export     — full / PDF / migrate (TBD until storage Export ships)
 *   - /settings/appearance — theme picker (System / Dark / Light)
 */
export function SettingsLayout() {
  return (
    <section data-testid="settings-page">
      <header className="page-header">
        <div>
          <h1>Settings</h1>
          <p>Configure storage, emergency access, cases, and exports.</p>
        </div>
      </header>

      <nav className="tabs-pill" style={{ marginBottom: 16, flexWrap: "wrap" }} aria-label="Settings sub-tabs">
        <SettingsLink to="/settings/storage" label="Storage" />
        <SettingsLink to="/settings/emergency" label="Emergency" />
        <SettingsLink to="/settings/cases" label="Cases" />
        <SettingsLink to="/settings/delegates" label="Delegates" />
        <SettingsLink to="/settings/export" label="Export" />
        <SettingsLink to="/settings/appearance" label="Appearance" />
      </nav>

      <Outlet />
    </section>
  );
}

function SettingsLink({ to, label }: { to: string; label: string }) {
  return (
    <NavLink
      to={to}
      style={{ display: "inline-flex" }}
      end={false}
    >
      {({ isActive }) => (
        <span
          className={isActive ? "active" : ""}
          style={{
            display: "inline-block",
            padding: "6px 12px",
            color: isActive ? "var(--color-ink)" : "var(--color-muted)",
            background: isActive ? "var(--color-surface-3)" : "transparent",
            fontSize: 12,
          }}
        >
          {label}
        </span>
      )}
    </NavLink>
  );
}
