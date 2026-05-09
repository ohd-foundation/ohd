import { Navigate, Route, Routes } from "react-router-dom";
import { AppShell } from "./components/AppShell";
import { ToastProvider } from "./components/Toast";
import { LogPage } from "./pages/LogPage";
import { DashboardPage } from "./pages/DashboardPage";
import { GrantsPage } from "./pages/GrantsPage";
import { PendingPage } from "./pages/PendingPage";
import { PendingQueriesPage } from "./pages/PendingQueriesPage";
import { LoginPage } from "./pages/LoginPage";
import { OidcCallbackPage } from "./pages/OidcCallbackPage";
import { SettingsLayout } from "./pages/settings/SettingsLayout";
import { StorageSettingsPage } from "./pages/settings/StorageSettingsPage";
import { EmergencySettingsPage } from "./pages/settings/EmergencySettingsPage";
import { CasesSettingsPage } from "./pages/settings/CasesSettingsPage";
import { DelegatesSettingsPage } from "./pages/settings/DelegatesSettingsPage";
import { ExportSettingsPage } from "./pages/settings/ExportSettingsPage";
import { AppearanceSettingsPage } from "./pages/settings/AppearanceSettingsPage";
import { useBootstrap } from "./ohdc/useStore";
import type { ReactNode } from "react";

/**
 * OHD Connect web — personal-side SPA.
 *
 * Routes (mirrored on desktop sidebar + mobile bottom-bar):
 *   - `/log`             — quick-entry tile grid (8 measurement / log types).
 *   - `/dashboard`       — recent events + per-channel sparklines.
 *   - `/grants`          — list, create-from-template, revoke; per-grant audit (TBD).
 *   - `/pending`         — write-with-approval review queue.
 *   - `/pending-queries` — read-with-approval queue (require_approval_per_query).
 *   - `/settings`        — storage, emergency, cases, export, appearance sub-pages.
 *
 * Auth gate: when there's no `ohds_…` token in sessionStorage and the
 * current URL has no `?token=` param, every route routes to `/no-token`
 * which renders a paste-token form. Once a token lands, `bootstrap()`
 * re-runs and the requested route resumes.
 *
 * v0 deferred:
 *   - Real OAuth code+PKCE login (storage hasn't shipped /authorize+/token).
 *   - AuditQuery surfaces (storage RPC stubbed).
 *   - Export (storage RPC stubbed).
 *   - Charts denser than sparklines (needs ReadSamples).
 */
export function App() {
  const { ready, error } = useBootstrap();
  return (
    <ToastProvider>
      <Routes>
        <Route element={<AppShell />}>
          <Route
            path="/"
            element={
              error === "no_token" ? (
                <Navigate to="/no-token" replace />
              ) : (
                <Navigate to="/log" replace />
              )
            }
          />
          <Route path="/no-token" element={<NoToken />} />
          <Route path="/login" element={<LoginPage />} />
          <Route path="/oidc-callback" element={<OidcCallbackPage />} />

          <Route
            path="/log"
            element={
              <BootstrapGate ready={ready} error={error}>
                <LogPage />
              </BootstrapGate>
            }
          />
          <Route
            path="/dashboard"
            element={
              <BootstrapGate ready={ready} error={error}>
                <DashboardPage />
              </BootstrapGate>
            }
          />
          <Route
            path="/grants"
            element={
              <BootstrapGate ready={ready} error={error}>
                <GrantsPage />
              </BootstrapGate>
            }
          />
          <Route
            path="/pending"
            element={
              <BootstrapGate ready={ready} error={error}>
                <PendingPage />
              </BootstrapGate>
            }
          />
          <Route
            path="/pending-queries"
            element={
              <BootstrapGate ready={ready} error={error}>
                <PendingQueriesPage />
              </BootstrapGate>
            }
          />
          <Route
            path="/settings"
            element={
              <BootstrapGate ready={ready} error={error}>
                <SettingsLayout />
              </BootstrapGate>
            }
          >
            <Route index element={<Navigate to="storage" replace />} />
            <Route path="storage" element={<StorageSettingsPage />} />
            <Route path="emergency" element={<EmergencySettingsPage />} />
            <Route path="cases" element={<CasesSettingsPage />} />
            <Route path="delegates" element={<DelegatesSettingsPage />} />
            <Route path="export" element={<ExportSettingsPage />} />
            <Route path="appearance" element={<AppearanceSettingsPage />} />
          </Route>

          <Route path="*" element={<NotFound />} />
        </Route>
      </Routes>
    </ToastProvider>
  );
}

function BootstrapGate({
  ready,
  error,
  children,
}: {
  ready: boolean;
  error: string | null;
  children: ReactNode;
}) {
  if (error === "no_token") {
    return <NoToken />;
  }
  if (!ready) {
    return (
      <div className="empty">
        <p>Loading from storage…</p>
      </div>
    );
  }
  if (error) {
    return (
      <div className="empty">
        <h3>Couldn't reach storage</h3>
        <p className="muted mono" style={{ fontSize: 12 }}>
          {error}
        </p>
        <p>
          Check the URL on the <a href="/settings/storage">Storage settings</a> page, or run a
          local <code>ohd-storage-server serve</code> on{" "}
          <code>http://localhost:8443</code>.
        </p>
      </div>
    );
  }
  return <>{children}</>;
}

function NoToken() {
  return (
    <div className="empty">
      <h2>Sign in</h2>
      <p>
        OHD Connect needs a self-session token (<code>ohds_…</code>) to talk to
        your storage instance.
      </p>
      <p>
        <a href="/login" className="btn btn-primary">
          Sign in with your storage instance
        </a>
      </p>
      <p className="muted" style={{ fontSize: 12 }}>
        Or paste a token on the <a href="/settings/storage">Storage settings</a>{" "}
        page, or open the app with a token in the URL:{" "}
        <code>http://localhost:5174/?token=ohds_…</code>. Mint one with{" "}
        <code>ohd-storage-server issue-self-token --db &lt;path&gt;</code>.
      </p>
    </div>
  );
}

function NotFound() {
  return (
    <div className="empty">
      <h2>404</h2>
      <p>No such route.</p>
      <p>
        <a href="/log">Back to Log</a>
      </p>
    </div>
  );
}
