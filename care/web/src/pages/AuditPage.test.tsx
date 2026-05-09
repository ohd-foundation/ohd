// Mount tests for AuditPage's two-sided JOIN.
//
// We mock `auditQuery` from the OHDC client to return a deterministic
// set of patient-side rows, and seed the operator-side audit log via
// `appendOperatorAuditEntry`. The page should:
//   - render the joined rows (matched + asymmetric),
//   - highlight asymmetries with the badge,
//   - count matched / storage-only / operator-only correctly,
//   - export a CSV download when "Export CSV" is clicked.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { ToastProvider } from "../components/Toast";
import {
  appendOperatorAuditEntry,
  clearOperatorAudit,
  type OperatorAuditEntry,
} from "../ohdc/operatorAudit";
import {
  canonicalFilterJson,
  canonicalQueryHash,
} from "../ohdc/canonicalQueryHash";

// Mock the OHDC client. We only need the `auditQuery` function and the
// AuditEntry / Ulid shapes from the gen/ proto.
const mockAuditQuery = vi.fn();
vi.mock("../ohdc/client", async () => {
  const actual = await vi.importActual<typeof import("../ohdc/client")>(
    "../ohdc/client",
  );
  return {
    ...actual,
    auditQuery: (...args: unknown[]) => mockAuditQuery(...args),
  };
});

import { AuditPage } from "./AuditPage";

function makeStorageRow(opts: {
  tsMs: number;
  action: string;
  queryKind: string;
  paramsJson: string;
  result?: string;
  rowsReturned?: number;
  rowsFiltered?: number;
}): {
  $typeName: "ohdc.v1.AuditEntry";
  tsMs: bigint;
  actorType: string;
  action: string;
  queryKind: string;
  queryParamsJson: string;
  result: string;
  rowsReturned?: bigint;
  rowsFiltered?: bigint;
} {
  return {
    $typeName: "ohdc.v1.AuditEntry",
    tsMs: BigInt(opts.tsMs),
    actorType: "grant",
    action: opts.action,
    queryKind: opts.queryKind,
    queryParamsJson: opts.paramsJson,
    result: opts.result ?? "success",
    rowsReturned: opts.rowsReturned != null ? BigInt(opts.rowsReturned) : undefined,
    rowsFiltered: opts.rowsFiltered != null ? BigInt(opts.rowsFiltered) : undefined,
  };
}

function mount() {
  return render(
    <ToastProvider>
      <MemoryRouter>
        <AuditPage />
      </MemoryRouter>
    </ToastProvider>,
  );
}

describe("AuditPage — two-sided JOIN", () => {
  beforeEach(() => {
    mockAuditQuery.mockReset();
    clearOperatorAudit();
  });

  it("renders matched rows + storage-only asymmetry", async () => {
    const now = Date.now();
    // The canonical query-params JSON the operator side will emit for a
    // bare query_events call. canonicalFilterJson is what we use on the
    // wire side; it must match what storage records.
    const filter = {
      eventTypesIn: ["std.heart_rate_resting"],
      includeSuperseded: true,
    };
    const paramsJson = canonicalFilterJson(filter);
    // The hash both sides will compute.
    const hash = await canonicalQueryHash("query_events", filter);

    // One storage row matches our operator-side audit row (same hash).
    // A second storage row has no operator counterpart — surfaces as
    // "storage-only" asymmetry.
    mockAuditQuery.mockResolvedValueOnce([
      makeStorageRow({
        tsMs: now - 1000,
        action: "read",
        queryKind: "query_events",
        paramsJson,
        rowsReturned: 12,
        rowsFiltered: 0,
      }),
      makeStorageRow({
        tsMs: now - 2000,
        action: "read",
        queryKind: "query_events",
        // Different filter → different hash → no match in operator log.
        paramsJson: JSON.stringify({ ...filter, eventTypesIn: ["std.symptom"] }),
        rowsReturned: 5,
        rowsFiltered: 2,
      }),
    ]);

    // Seed operator audit with the matching row.
    const opEntry: OperatorAuditEntry = {
      tsMs: now - 1000,
      operatorSubject: "op-123",
      grantUlid: "01HZZZ",
      ohdcAction: "query_events",
      queryHash: hash,
      queryKind: "query_events",
      result: "success",
      rowsReturned: 12,
      rowsFiltered: null,
      reason: null,
    };
    appendOperatorAuditEntry(opEntry);

    mount();

    await waitFor(() =>
      expect(screen.getByTestId("audit-page")).toBeInTheDocument(),
    );
    await waitFor(() =>
      expect(screen.getByTestId("audit-table")).toBeInTheDocument(),
    );

    // Two storage-side rows — they're both rendered.
    const rows = screen
      .getAllByRole("row")
      .filter((r) => r.getAttribute("data-asymmetry") !== null);
    expect(rows.length).toBe(2);

    // First row is matched, second is storage-only.
    const matched = rows.find(
      (r) => r.getAttribute("data-asymmetry") === "matched",
    );
    const storageOnly = rows.find(
      (r) => r.getAttribute("data-asymmetry") === "storage_only",
    );
    expect(matched).toBeDefined();
    expect(storageOnly).toBeDefined();

    // Asymmetry pill counts.
    expect(screen.getByTestId("audit-asym-storage")).toHaveTextContent(
      "1 storage-only",
    );
    expect(screen.getByTestId("audit-asym-operator")).toHaveTextContent(
      "0 operator-only",
    );

    // Filtered-rows column highlights when rows_filtered > 0.
    expect(screen.getByText(/2 filtered/)).toBeInTheDocument();
  });

  it("operator-only asymmetry: an operator row without a storage counterpart", async () => {
    const now = Date.now();
    mockAuditQuery.mockResolvedValueOnce([]);

    appendOperatorAuditEntry({
      tsMs: now - 500,
      operatorSubject: "op-123",
      grantUlid: "01HZZZ",
      ohdcAction: "query_events",
      queryHash: "deadbeef",
      queryKind: "query_events",
      result: "success",
      rowsReturned: 7,
      rowsFiltered: null,
      reason: null,
    });

    mount();
    await waitFor(() =>
      expect(screen.getByTestId("audit-page")).toBeInTheDocument(),
    );
    await waitFor(() =>
      expect(screen.getByTestId("audit-asym-operator")).toHaveTextContent(
        "1 operator-only",
      ),
    );
    expect(screen.getByTestId("audit-asym-storage")).toHaveTextContent(
      "0 storage-only",
    );
    // An asymmetry badge should be visible (the "no storage row" pill).
    expect(screen.getByText("no storage row")).toBeInTheDocument();
  });

  it("filter chip changes re-fire auditQuery", async () => {
    mockAuditQuery.mockResolvedValue([]);
    mount();
    await waitFor(() => expect(mockAuditQuery).toHaveBeenCalled());
    const callsBefore = mockAuditQuery.mock.calls.length;

    const user = userEvent.setup();
    await user.click(screen.getByTestId("audit-window-7d"));
    await waitFor(() =>
      expect(mockAuditQuery.mock.calls.length).toBeGreaterThan(callsBefore),
    );
    // The latest call's `fromMs` should be ~7 days back.
    const lastCall = mockAuditQuery.mock.calls.at(-1)![0] as { fromMs?: number };
    expect(lastCall.fromMs).toBeGreaterThan(0);
    expect(Date.now() - (lastCall.fromMs ?? 0)).toBeGreaterThan(6 * 24 * 60 * 60 * 1000);
  });

  it("export CSV button enables once rows are loaded", async () => {
    mockAuditQuery.mockResolvedValueOnce([
      makeStorageRow({
        tsMs: Date.now(),
        action: "read",
        queryKind: "query_events",
        paramsJson: "{}",
      }),
    ]);
    mount();
    const exportBtn = await screen.findByTestId("audit-export-csv");
    await waitFor(() => expect(exportBtn).not.toBeDisabled());
  });
});
