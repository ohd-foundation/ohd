import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { App } from "../App";

function renderAt(path: string) {
  return render(
    <MemoryRouter initialEntries={[path]}>
      <App />
    </MemoryRouter>,
  );
}

describe("OHD Dispatch v0 — smoke", () => {
  it("renders the shell with the brand mark", () => {
    renderAt("/active");
    expect(screen.getByText("OHD")).toBeInTheDocument();
    expect(screen.getByText(/EMERGENCY · DISPATCH/i)).toBeInTheDocument();
  });

  it("redirects / to /active and renders the active cases page", () => {
    renderAt("/");
    expect(screen.getByTestId("active-cases-page")).toBeInTheDocument();
    // At least one mock case row is visible.
    expect(screen.getByText("RUN-7841")).toBeInTheDocument();
  });

  it("renders the active cases page directly", () => {
    renderAt("/active");
    expect(screen.getByTestId("active-cases-page")).toBeInTheDocument();
    // Metric tiles are present.
    expect(screen.getByText(/Open$/)).toBeInTheDocument();
    expect(screen.getByText(/In handoff/)).toBeInTheDocument();
  });

  it("renders the crew roster page", () => {
    renderAt("/roster");
    expect(screen.getByTestId("crew-roster-page")).toBeInTheDocument();
    // Mock crew name shows up.
    expect(screen.getByText("P. Horak")).toBeInTheDocument();
  });

  it("renders the audit page with the TBD banner", () => {
    renderAt("/audit");
    expect(screen.getByTestId("audit-page")).toBeInTheDocument();
    expect(screen.getByText(/TBD:/)).toBeInTheDocument();
    expect(screen.getByText(/AuditQuery/)).toBeInTheDocument();
  });

  it("renders the operator records page", () => {
    renderAt("/records");
    expect(screen.getByTestId("operator-records-page")).toBeInTheDocument();
    expect(screen.getByText("RUN-7831")).toBeInTheDocument();
  });

  it("renders the settings page with cert info", () => {
    renderAt("/settings");
    expect(screen.getByTestId("settings-page")).toBeInTheDocument();
    // Cert section header (h2) is unique even though "authority cert" also
    // appears in field hint copy.
    expect(
      screen.getByRole("heading", { name: /Authority cert/ }),
    ).toBeInTheDocument();
    expect(screen.getByText(/sha256:/)).toBeInTheDocument();
  });

  it("renders the OIDC login page with a sign-in form", () => {
    renderAt("/login");
    expect(screen.getByTestId("login-page")).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: /Sign in to dispatch/ }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /Sign in with operator IdP/ }),
    ).toBeInTheDocument();
  });
});
