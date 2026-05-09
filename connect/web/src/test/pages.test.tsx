import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter, Routes, Route } from "react-router-dom";
import { ToastProvider } from "../components/Toast";
import { LogPage } from "../pages/LogPage";
import { DashboardPage } from "../pages/DashboardPage";
import { GrantsPage } from "../pages/GrantsPage";
import { PendingPage } from "../pages/PendingPage";
import { PendingQueriesPage } from "../pages/PendingQueriesPage";
import { SettingsLayout } from "../pages/settings/SettingsLayout";
import { StorageSettingsPage } from "../pages/settings/StorageSettingsPage";
import { EmergencySettingsPage } from "../pages/settings/EmergencySettingsPage";
import { CasesSettingsPage } from "../pages/settings/CasesSettingsPage";
import { DelegatesSettingsPage } from "../pages/settings/DelegatesSettingsPage";
import { ExportSettingsPage } from "../pages/settings/ExportSettingsPage";
import { AppearanceSettingsPage } from "../pages/settings/AppearanceSettingsPage";
import { _resetForTesting, _setSnapshotForTesting } from "../ohdc/store";
import {
  _clearPendingQueriesMock,
  _seedPendingQueriesMock,
  _readPendingQueriesMock,
  type PendingQuery,
} from "../ohdc/client";
import { act, fireEvent } from "@testing-library/react";
import { THEME_STORAGE_KEY } from "../theme";

beforeEach(() => {
  _resetForTesting();
  _clearPendingQueriesMock();
  // Pretend bootstrap finished cleanly with empty data so pages mount.
  _setSnapshotForTesting({
    ready: true,
    error: null,
    me: null,
    events: [],
    grants: [],
    pending: [],
    pendingQueries: [],
    cases: [],
    health: { status: "ok", serverVersion: "0.0.0", protocolVersion: "ohdc.v0", serverTimeMs: Date.now() },
  });
  if (typeof window !== "undefined") {
    sessionStorage.clear();
    localStorage.clear();
    // Reset html data-theme that earlier tests may have set.
    if (document?.documentElement) {
      delete document.documentElement.dataset.theme;
    }
  }
});

function makePendingQuery(overrides: Partial<PendingQuery> = {}): PendingQuery {
  return {
    queryUlid: overrides.queryUlid ?? "01J0PENDQUERYTEST0000000001",
    grantUlid: overrides.grantUlid ?? "01J0DEMOGRANT00000000000001",
    grantLabel: overrides.grantLabel ?? "Test grant",
    queryKind: overrides.queryKind ?? "query_events",
    summary: overrides.summary ?? {
      eventTypes: ["std.blood_glucose"],
      fromMs: Date.now() - 7 * 86_400_000,
      toMs: null,
      hint: "Wants glucose, last 7 days",
    },
    requestedAtMs: overrides.requestedAtMs ?? Date.now() - 60_000,
    expiresAtMs: overrides.expiresAtMs ?? Date.now() + 86_400_000,
  };
}

function withProviders(ui: React.ReactNode, initialPath = "/") {
  return (
    <MemoryRouter initialEntries={[initialPath]}>
      <ToastProvider>{ui}</ToastProvider>
    </MemoryRouter>
  );
}

describe("OHD Connect — page mount tests", () => {
  it("LogPage mounts with all 9 tile buttons", () => {
    render(withProviders(<LogPage />));
    expect(screen.getByTestId("log-page")).toBeInTheDocument();
    expect(screen.getByTestId("tile-glucose")).toBeInTheDocument();
    expect(screen.getByTestId("tile-heart_rate")).toBeInTheDocument();
    expect(screen.getByTestId("tile-blood_pressure")).toBeInTheDocument();
    expect(screen.getByTestId("tile-temperature")).toBeInTheDocument();
    expect(screen.getByTestId("tile-medication")).toBeInTheDocument();
    expect(screen.getByTestId("tile-symptom")).toBeInTheDocument();
    expect(screen.getByTestId("tile-meal")).toBeInTheDocument();
    expect(screen.getByTestId("tile-mood")).toBeInTheDocument();
    expect(screen.getByTestId("tile-note")).toBeInTheDocument();
  });

  it("DashboardPage mounts and shows empty state without events", () => {
    render(withProviders(<DashboardPage />));
    expect(screen.getByTestId("dashboard-page")).toBeInTheDocument();
    expect(screen.getByText(/Nothing yet/i)).toBeInTheDocument();
  });

  it("GrantsPage mounts with create button and empty state", () => {
    render(withProviders(<GrantsPage />));
    expect(screen.getByTestId("grants-page")).toBeInTheDocument();
    expect(screen.getByTestId("create-grant")).toBeInTheDocument();
    expect(screen.getByText(/No active grants/i)).toBeInTheDocument();
  });

  it("PendingPage mounts with empty state", () => {
    render(withProviders(<PendingPage />));
    expect(screen.getByTestId("pending-page")).toBeInTheDocument();
    expect(screen.getByText(/No pending submissions/i)).toBeInTheDocument();
  });

  it("Settings storage sub-page mounts", () => {
    render(
      withProviders(
        <Routes>
          <Route element={<SettingsLayout />}>
            <Route path="/settings/storage" element={<StorageSettingsPage />} />
          </Route>
        </Routes>,
        "/settings/storage",
      ),
    );
    expect(screen.getByTestId("settings-page")).toBeInTheDocument();
    expect(screen.getByTestId("settings-storage")).toBeInTheDocument();
  });

  it("Settings emergency sub-page mounts with the feature toggle", () => {
    render(
      withProviders(
        <Routes>
          <Route element={<SettingsLayout />}>
            <Route path="/settings/emergency" element={<EmergencySettingsPage />} />
          </Route>
        </Routes>,
        "/settings/emergency",
      ),
    );
    expect(screen.getByTestId("settings-emergency")).toBeInTheDocument();
    expect(screen.getByLabelText(/Enable emergency access/i)).toBeInTheDocument();
  });

  it("Settings cases sub-page mounts", () => {
    render(
      withProviders(
        <Routes>
          <Route element={<SettingsLayout />}>
            <Route path="/settings/cases" element={<CasesSettingsPage />} />
          </Route>
        </Routes>,
        "/settings/cases",
      ),
    );
    expect(screen.getByTestId("settings-cases")).toBeInTheDocument();
  });

  it("Settings delegates sub-page mounts with empty state", () => {
    render(
      withProviders(
        <Routes>
          <Route element={<SettingsLayout />}>
            <Route path="/settings/delegates" element={<DelegatesSettingsPage />} />
          </Route>
        </Routes>,
        "/settings/delegates",
      ),
    );
    expect(screen.getByTestId("settings-delegates")).toBeInTheDocument();
    expect(screen.getByTestId("issue-delegate")).toBeInTheDocument();
    expect(screen.getByText(/No delegates configured/i)).toBeInTheDocument();
  });

  it("Settings delegates lists existing delegate grants with badge", () => {
    // Inject one delegate-kind grant into the snapshot. Cast through
    // `unknown` so the test doesn't need to construct every protobuf
    // schema field.
    const fakeGrant = {
      $typeName: "ohdc.v0.Grant",
      ulid: { $typeName: "ohdc.v0.Ulid", bytes: new Uint8Array(16) },
      granteeLabel: "Mom's caregiver",
      granteeKind: "delegate",
      purpose: "delegate_identity:OIDC...",
      createdAtMs: BigInt(Date.now() - 86_400_000),
      expiresAtMs: BigInt(Date.now() + 90 * 86_400_000),
      revokedAtMs: BigInt(0),
      approvalMode: "always",
      defaultAction: "allow",
      eventTypeRules: [],
      writeEventTypeRules: [],
      sensitivityRules: [],
      autoApproveEventTypes: [],
      notifyOnAccess: true,
      stripNotes: false,
      aggregationOnly: false,
      requireApprovalPerQuery: false,
      lastUsedMs: BigInt(0),
      useCount: BigInt(0),
    };
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    _setSnapshotForTesting({ grants: [fakeGrant as any] });
    render(
      withProviders(
        <Routes>
          <Route element={<SettingsLayout />}>
            <Route path="/settings/delegates" element={<DelegatesSettingsPage />} />
          </Route>
        </Routes>,
        "/settings/delegates",
      ),
    );
    expect(screen.getByTestId("settings-delegates")).toBeInTheDocument();
    expect(screen.getAllByTestId("delegate-badge").length).toBeGreaterThan(0);
    expect(screen.getByText(/Mom's caregiver/)).toBeInTheDocument();
  });

  it("Settings export sub-page mounts", () => {
    render(
      withProviders(
        <Routes>
          <Route element={<SettingsLayout />}>
            <Route path="/settings/export" element={<ExportSettingsPage />} />
          </Route>
        </Routes>,
        "/settings/export",
      ),
    );
    expect(screen.getByTestId("settings-export")).toBeInTheDocument();
  });

  // ----- F. Pending read queries page (require_approval_per_query) -----

  it("PendingQueriesPage mounts with empty state", () => {
    render(withProviders(<PendingQueriesPage />));
    expect(screen.getByTestId("pending-queries-page")).toBeInTheDocument();
    expect(screen.getByText(/No pending read queries/i)).toBeInTheDocument();
    // Bulk action bar should be absent without selection.
    expect(screen.queryByTestId("bulk-action-bar")).not.toBeInTheDocument();
  });

  it("PendingQueriesPage shows mock-mode banner when wire RPCs aren't exposed", () => {
    // Today the proto doesn't expose pending_queries RPCs, so the mock
    // path is active and the banner should render.
    render(withProviders(<PendingQueriesPage />));
    expect(screen.getByTestId("pending-queries-mock-banner")).toBeInTheDocument();
  });

  it("PendingQueriesPage renders rows from the snapshot and supports bulk-approve", async () => {
    const r1 = makePendingQuery({ queryUlid: "01J0PENDQUERYTEST0000000001", grantLabel: "Doc A" });
    const r2 = makePendingQuery({ queryUlid: "01J0PENDQUERYTEST0000000002", grantLabel: "Researcher B" });
    // Seed both the mock store (so approve/reject calls succeed) and the
    // store snapshot (so the page renders rows immediately).
    _seedPendingQueriesMock([r1, r2]);
    _setSnapshotForTesting({ pendingQueries: [r1, r2] });

    render(withProviders(<PendingQueriesPage />));
    expect(screen.getAllByTestId("pending-query-card").length).toBe(2);
    expect(screen.getByText(/Doc A/)).toBeInTheDocument();
    expect(screen.getByText(/Researcher B/)).toBeInTheDocument();

    // Pick "select all", then bulk-approve. Verify the mock got drained.
    fireEvent.click(screen.getByTestId("select-all-queries"));
    expect(screen.getByTestId("bulk-action-bar")).toBeInTheDocument();
    await act(async () => {
      fireEvent.click(screen.getByTestId("bulk-approve"));
      // Let the await chain in bulkApprovePendingQueries + refresh settle.
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
    expect(_readPendingQueriesMock().length).toBe(0);
  });

  // ----- G. Appearance settings (theme toggle) -----

  it("Settings appearance sub-page mounts with three radio options", () => {
    render(
      withProviders(
        <Routes>
          <Route element={<SettingsLayout />}>
            <Route path="/settings/appearance" element={<AppearanceSettingsPage />} />
          </Route>
        </Routes>,
        "/settings/appearance",
      ),
    );
    expect(screen.getByTestId("settings-appearance")).toBeInTheDocument();
    expect(screen.getByTestId("theme-system")).toBeInTheDocument();
    expect(screen.getByTestId("theme-dark")).toBeInTheDocument();
    expect(screen.getByTestId("theme-light")).toBeInTheDocument();
  });

  it("Selecting Light theme persists and sets data-theme on <html>", () => {
    render(
      withProviders(
        <Routes>
          <Route element={<SettingsLayout />}>
            <Route path="/settings/appearance" element={<AppearanceSettingsPage />} />
          </Route>
        </Routes>,
        "/settings/appearance",
      ),
    );

    fireEvent.click(screen.getByTestId("theme-light"));
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("light");

    fireEvent.click(screen.getByTestId("theme-dark"));
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("dark");
  });
});
