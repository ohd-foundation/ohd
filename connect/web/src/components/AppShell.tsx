import { NavLink, Outlet } from "react-router-dom";
import { useStoreVersion } from "../ohdc/useStore";
import { getMyUserUlid, getSnapshot, signOut } from "../ohdc/store";

/**
 * Top-level layout. Renders:
 *   - Sticky top bar with brand, page title slot, and a connectivity dot.
 *   - Sidebar navigation on desktop (>=880px).
 *   - Bottom-bar navigation on mobile (<880px).
 *   - Outlet for the page content.
 *
 * The bottom-bar is the personal-app's primary navigation per the Connect
 * Android design (`android/app/src/main/java/com/ohd/connect/MainActivity.kt`)
 * and the Connect web style brief in the user spec.
 */
export function AppShell() {
  useStoreVersion(); // re-render on snapshot bump (header status badges)
  const snap = getSnapshot();
  const userUlid = getMyUserUlid();
  const pendingCount = snap.pending.length;
  const pendingQueriesCount = snap.pendingQueries.length;

  const dotClass =
    snap.error === "no_token"
      ? "warn"
      : snap.error
      ? "danger"
      : snap.health?.status === "ok"
      ? ""
      : "warn";
  const dotTitle =
    snap.error === "no_token"
      ? "No session token"
      : snap.error
      ? snap.error
      : snap.health?.status === "ok"
      ? `Connected · v${snap.health.serverVersion}`
      : "Storage not yet reached";

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="topbar-brand">
          <span className="topbar-brand-name">OHD Connect</span>
          <span className="topbar-brand-tag">v0</span>
        </div>
        <div className="topbar-title">
          {userUlid ? (
            <span className="mono" title={userUlid}>
              {userUlid.slice(0, 8)}…
            </span>
          ) : null}
        </div>
        <div className="topbar-status" title={dotTitle}>
          <span className={`status-dot ${dotClass}`.trim()} />
          <span>
            {snap.health?.status === "ok" ? "online" : snap.error === "no_token" ? "offline" : "…"}
          </span>
          {snap.me ? (
            <button
              type="button"
              className="btn btn-ghost btn-sm"
              onClick={() => {
                signOut();
                window.location.href = "/";
              }}
              aria-label="Sign out"
            >
              Sign out
            </button>
          ) : null}
        </div>
      </header>

      <div className="shell-body">
        <nav className="sidebar" aria-label="Primary">
          <SidebarLink to="/log" icon="◯" label="Log" />
          <SidebarLink to="/dashboard" icon="◈" label="Dashboard" />
          <SidebarLink to="/grants" icon="◇" label="Grants" />
          <SidebarLink to="/pending" icon="△" label="Pending writes" badge={pendingCount} />
          <SidebarLink
            to="/pending-queries"
            icon="▽"
            label="Pending reads"
            badge={pendingQueriesCount}
          />
          <SidebarLink to="/settings" icon="◎" label="Settings" />
        </nav>

        <main className="main">
          <div className="main-inner">
            <Outlet />
          </div>
        </main>
      </div>

      <nav className="bottom-bar" aria-label="Primary">
        <BottomLink to="/log" icon="◯" label="Log" />
        <BottomLink to="/dashboard" icon="◈" label="Dashboard" />
        <BottomLink to="/grants" icon="◇" label="Grants" />
        <BottomLink
          to="/pending"
          icon="△"
          label="Writes"
          badge={pendingCount}
        />
        <BottomLink
          to="/pending-queries"
          icon="▽"
          label="Reads"
          badge={pendingQueriesCount}
        />
        <BottomLink to="/settings" icon="◎" label="Settings" />
      </nav>
    </div>
  );
}

function SidebarLink({ to, icon, label, badge }: { to: string; icon: string; label: string; badge?: number }) {
  return (
    <NavLink to={to} className={({ isActive }) => (isActive ? "active" : "")}>
      <span aria-hidden="true">{icon}</span>
      <span>{label}</span>
      {badge ? <span className="badge-pill">{badge}</span> : null}
    </NavLink>
  );
}

function BottomLink({ to, icon, label, badge }: { to: string; icon: string; label: string; badge?: number }) {
  return (
    <NavLink to={to} className={({ isActive }) => (isActive ? "active" : "")}>
      <span className="icon" aria-hidden="true">
        {icon}
      </span>
      <span>{label}</span>
      {badge ? <span className="badge-pill">{badge}</span> : null}
    </NavLink>
  );
}
