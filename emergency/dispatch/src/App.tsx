import { Navigate, Route, Routes } from "react-router-dom";
import { AppShell } from "./components/AppShell";
import { ActiveCasesPage } from "./pages/ActiveCasesPage";
import { CrewRosterPage } from "./pages/CrewRosterPage";
import { AuditPage } from "./pages/AuditPage";
import { OperatorRecordsPage } from "./pages/OperatorRecordsPage";
import { SettingsPage } from "./pages/SettingsPage";
import { LoginPage } from "./pages/LoginPage";
import { OidcCallbackPage } from "./pages/OidcCallbackPage";
import { useBootstrap } from "./ohdc/useStore";
import { isMockMode } from "./mock/store";

/**
 * OHD Emergency dispatch console v0 — operator-side SPA.
 *
 * Five sections in the sidebar:
 *   /active   — live case board (default)
 *   /roster   — crew roster (operator-side state)
 *   /audit    — break-glass + operator-action log
 *   /records  — operator records DB browser
 *   /settings — storage URL, operator token, station label, cert info
 *
 * Auth: operator-session bearer (from ?token= or settings page) with the
 * standard `Authorization: Bearer <token>` header on every OHDC call.
 * Distinct from care/web's grant token — see ohdc/client.ts.
 */
export function App() {
  const { ready, error } = useBootstrap();

  return (
    <Routes>
      <Route element={<AppShell />}>
        <Route path="/" element={<Navigate to="/active" replace />} />
        <Route path="/login" element={<LoginPage />} />
        <Route path="/oidc-callback" element={<OidcCallbackPage />} />
        <Route
          path="/active"
          element={<BootstrapGate ready={ready} error={error}><ActiveCasesPage /></BootstrapGate>}
        />
        <Route path="/roster" element={<CrewRosterPage />} />
        <Route path="/audit" element={<AuditPage />} />
        <Route path="/records" element={<OperatorRecordsPage />} />
        <Route path="/settings" element={<SettingsPage />} />
        <Route path="*" element={<NotFound />} />
      </Route>
    </Routes>
  );
}

function BootstrapGate({
  ready,
  error,
  children,
}: {
  ready: boolean;
  error: string | null;
  children: React.ReactNode;
}) {
  // Mock mode bypasses the gate — pages render against MOCK_* directly.
  if (isMockMode) return <>{children}</>;

  if (error === "no_token") {
    return (
      <div className="page">
        <div className="banner banner-warn">
          <strong>No operator token.</strong> Open this app with{" "}
          <code>?token=…</code>, or paste one in <a href="/settings">Settings</a>.
        </div>
        {children}
      </div>
    );
  }
  if (!ready) {
    return (
      <div className="page">
        <div className="empty">Loading from storage…</div>
      </div>
    );
  }
  if (error) {
    return (
      <div className="page">
        <div className="banner banner-error">
          <strong>Storage unavailable.</strong>{" "}
          <span className="mono">{error}</span>
        </div>
        {children}
      </div>
    );
  }
  return <>{children}</>;
}

function NotFound() {
  return (
    <div className="page">
      <div className="empty">
        <h2>404</h2>
        <p>No such route.</p>
        <a href="/active">Back to active cases</a>
      </div>
    </div>
  );
}
