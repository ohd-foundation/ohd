import { Navigate, Route, Routes } from "react-router-dom";
import { AppShell } from "./components/AppShell";
import { ToastProvider } from "./components/Toast";
import { RosterPage } from "./pages/RosterPage";
import { PatientPage } from "./pages/PatientPage";
import { TimelineTab } from "./pages/tabs/TimelineTab";
import { VitalsTab } from "./pages/tabs/VitalsTab";
import { MedicationsTab } from "./pages/tabs/MedicationsTab";
import { SymptomsTab } from "./pages/tabs/SymptomsTab";
import { FoodsTab } from "./pages/tabs/FoodsTab";
import { LabsTab } from "./pages/tabs/LabsTab";
import { ImagingTab } from "./pages/tabs/ImagingTab";
import { NotesTab } from "./pages/tabs/NotesTab";
import { LoginPage } from "./pages/LoginPage";
import { OidcCallbackPage } from "./pages/OidcCallbackPage";
import { PendingPage } from "./pages/PendingPage";
import { AuditPage } from "./pages/AuditPage";
import { ChatPage } from "./pages/ChatPage";
import { SettingsMcpPage } from "./pages/SettingsMcpPage";
import { useBootstrap } from "./ohdc/useStore";

/**
 * OHD Care v0 — operator-facing SPA.
 *
 * v0 scope:
 *   - Roster of mock patients with status indicators (last visit, flags, meds).
 *   - Per-patient view with header (active patient + grant scope + active case),
 *     visit-prep brief, tabs (Timeline / Vitals / Medications / Symptoms / Foods /
 *     Labs / Imaging / Notes), and per-tab "New entry" submission with the
 *     SPEC §3.3 confirmation step echoing the patient label.
 *   - Active patient pinned in the top bar, even on the roster route.
 *
 * Out of scope for v0 (placeholders / stubs):
 *   - Real OHDC client + grant vault wiring (Storage component owns the proto
 *     codegen; lands in a later phase).
 *   - Operator OIDC; "Sign out" is a stub.
 *   - Add-patient share-artifact import (paste/scan).
 *   - Audit transparency panel.
 *
 * Mock store at src/mock/store.ts.
 */
export function App() {
  // Kick off the OHDC bootstrap (WhoAmI + initial QueryEvents). Bootstrap
  // runs to completion once; the snapshot is then refreshed after every
  // write (see `submit*` in `ohdc/store.ts`). The status drives a small
  // gate banner so the user can tell whether the grant actually resolved.
  const { ready, error } = useBootstrap();
  return (
    <ToastProvider>
      <Routes>
        <Route element={<AppShell />}>
          <Route
            path="/"
            element={
              error === "no_token"
                ? <Navigate to="/no-grant" replace />
                : <Navigate to="/roster" replace />
            }
          />
          <Route
            path="/roster"
            element={<BootstrapGate ready={ready} error={error}><RosterPage /></BootstrapGate>}
          />
          <Route path="/no-grant" element={<NoGrant />} />
          <Route path="/login" element={<LoginPage />} />
          <Route path="/oidc-callback" element={<OidcCallbackPage />} />
          <Route
            path="/pending"
            element={<BootstrapGate ready={ready} error={error}><PendingPage /></BootstrapGate>}
          />
          <Route
            path="/audit"
            element={<BootstrapGate ready={ready} error={error}><AuditPage /></BootstrapGate>}
          />
          <Route
            path="/chat"
            element={<BootstrapGate ready={ready} error={error}><ChatPage /></BootstrapGate>}
          />
          <Route path="/settings/mcp" element={<SettingsMcpPage />} />
          <Route
            path="/patient/:label"
            element={<BootstrapGate ready={ready} error={error}><PatientPage /></BootstrapGate>}
          >
            <Route index element={<Navigate to="timeline" replace />} />
            <Route path="timeline" element={<TimelineTab />} />
            <Route path="vitals" element={<VitalsTab />} />
            <Route path="medications" element={<MedicationsTab />} />
            <Route path="symptoms" element={<SymptomsTab />} />
            <Route path="foods" element={<FoodsTab />} />
            <Route path="labs" element={<LabsTab />} />
            <Route path="imaging" element={<ImagingTab />} />
            <Route path="notes" element={<NotesTab />} />
          </Route>
          <Route path="*" element={<NotFound />} />
        </Route>
      </Routes>
    </ToastProvider>
  );
}

/** Renders children only when the OHDC bootstrap has completed. */
function BootstrapGate({
  ready,
  error,
  children,
}: {
  ready: boolean;
  error: string | null;
  children: React.ReactNode;
}) {
  if (error === "no_token") {
    return <NoGrant />;
  }
  if (!ready) {
    return (
      <div className="empty">
        <p>Loading patient data from storage…</p>
      </div>
    );
  }
  if (error) {
    return (
      <div className="empty">
        <h3>Could not load from storage</h3>
        <p className="muted mono" style={{ fontSize: 12 }}>{error}</p>
        <p>
          Make sure <code>ohd-storage-server serve</code> is running and the grant
          token in the URL is valid. See <code>care/demo/README.md</code>.
        </p>
      </div>
    );
  }
  return <>{children}</>;
}

function NoGrant() {
  return (
    <div className="empty">
      <h2>No grant token</h2>
      <p>
        Open this app with a grant token in the URL, e.g.<br />
        <code>http://localhost:5173/?token=ohdg_…</code>
      </p>
      <p>
        Issue one with{" "}
        <code>ohd-storage-server issue-grant-token --db /tmp/ohd-demo.db --read std.blood_glucose,std.heart_rate_resting,std.body_temperature,std.medication_dose,std.symptom --write std.clinical_note</code>
      </p>
    </div>
  );
}

function NotFound() {
  return (
    <div className="empty">
      <h2>404</h2>
      <p>No such route.</p>
      <a href="/roster">Back to roster</a>
    </div>
  );
}
