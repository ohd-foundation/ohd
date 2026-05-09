// Mount tests for PendingPage's bulk approve/reject flow.
//
// We mock the OHDC client functions so the test runs without a backend.
// The PendingPage component is what we want to assert on; the wire
// helpers in `client.ts` have their own integration story (see
// `care/cli` round-trip tests).

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { ToastProvider } from "../components/Toast";

// Mock the OHDC client used by PendingPage. The mock state is mutated by
// each test; restore in `beforeEach`.
const mockListPending = vi.fn();
const mockApprovePending = vi.fn();
const mockRejectPending = vi.fn();

vi.mock("../ohdc/client", () => ({
  listPending: () => mockListPending(),
  approvePending: (...args: unknown[]) => mockApprovePending(...args),
  rejectPending: (...args: unknown[]) => mockRejectPending(...args),
  // The component imports ulidToCrockford for rendering; pass-through.
  ulidToCrockford: (b: Uint8Array) => {
    // 26-char Crockford-base32 of 16 bytes; here we just return a stable
    // pseudo-id from the bytes so test assertions can match.
    return Array.from(b)
      .map((x) => x.toString(16).padStart(2, "0"))
      .join("")
      .toUpperCase()
      .slice(0, 26);
  },
}));

import { PendingPage } from "./PendingPage";

function ulidBytes(seed: number): Uint8Array {
  const out = new Uint8Array(16);
  for (let i = 0; i < 16; i++) out[i] = (seed + i) & 0xff;
  return out;
}

function makePending(seed: number, eventType: string) {
  return {
    ulid: { bytes: ulidBytes(seed) },
    submittedAtMs: BigInt(1_700_000_000_000 + seed * 1000),
    expiresAtMs: BigInt(1_700_086_400_000),
    status: "pending",
    event: {
      eventType,
      timestampMs: BigInt(1_700_000_000_000 + seed * 1000),
      channels: [{ channelPath: "value" }, { channelPath: "unit" }],
    },
  };
}

function mount() {
  return render(
    <ToastProvider>
      <MemoryRouter>
        <PendingPage />
      </MemoryRouter>
    </ToastProvider>,
  );
}

describe("PendingPage — bulk approve/reject", () => {
  beforeEach(() => {
    mockListPending.mockReset();
    mockApprovePending.mockReset();
    mockRejectPending.mockReset();
  });

  it("renders rows + selecting all enables bulk toolbar", async () => {
    mockListPending.mockResolvedValueOnce([
      makePending(1, "lab_result"),
      makePending(2, "clinical_note"),
      makePending(3, "lab_result"),
    ]);

    mount();

    await waitFor(() =>
      expect(screen.getByTestId("pending-page")).toBeInTheDocument(),
    );

    // Three rows visible.
    const rows = screen.getAllByRole("row");
    // 1 header row + 3 data rows.
    expect(rows.length).toBe(1 + 3);

    // Bulk toolbar isn't visible until something is selected.
    expect(screen.queryByTestId("bulk-toolbar")).toBeNull();

    // Click "Select all visible" — toolbar appears with N=3.
    const user = userEvent.setup();
    await user.click(screen.getByTestId("select-all"));
    expect(await screen.findByTestId("bulk-toolbar")).toBeInTheDocument();
    expect(screen.getByTestId("bulk-approve")).toHaveTextContent(
      "Approve selected (3)",
    );
    expect(screen.getByTestId("bulk-reject")).toHaveTextContent(
      "Reject selected (3)",
    );
  });

  it("approve flow: confirm + per-item progress + success toast", async () => {
    mockListPending.mockResolvedValueOnce([
      makePending(1, "lab_result"),
      makePending(2, "lab_result"),
    ]);
    mockApprovePending.mockResolvedValue({
      committedAtMs: 1,
      eventUlid: ulidBytes(99),
    });

    mount();
    await waitFor(() =>
      expect(screen.getByTestId("pending-page")).toBeInTheDocument(),
    );

    const user = userEvent.setup();
    await user.click(screen.getByTestId("select-all"));
    await user.click(screen.getByTestId("bulk-approve"));

    // Confirm dialog appears.
    const dialog = await screen.findByTestId("bulk-confirm");
    expect(dialog).toBeInTheDocument();
    expect(within(dialog).getByText(/Approve 2 submissions/)).toBeInTheDocument();

    // Refresh-after-batch reload returns the now-empty list.
    mockListPending.mockResolvedValueOnce([]);

    await user.click(screen.getByTestId("bulk-confirm-go"));

    // Each pending ULID approved exactly once.
    await waitFor(() => expect(mockApprovePending).toHaveBeenCalledTimes(2));
    // First call passes the auto-approve flag = false (no shortcut chosen).
    expect(mockApprovePending.mock.calls[0][1]).toBe(false);
    expect(mockApprovePending.mock.calls[1][1]).toBe(false);

    // Progress dialog reaches "done" state.
    const done = await screen.findByTestId("bulk-done");
    await user.click(done);

    // Toast shows the success.
    expect(await screen.findByText(/Approved 2 submissions/)).toBeInTheDocument();
  });

  it("approve & trust forever: passes alsoAutoApproveThisType=true on first call", async () => {
    mockListPending.mockResolvedValueOnce([
      makePending(1, "lab_result"),
      makePending(2, "lab_result"),
    ]);
    mockApprovePending.mockResolvedValue({ committedAtMs: 1 });

    mount();
    await waitFor(() =>
      expect(screen.getByTestId("pending-page")).toBeInTheDocument(),
    );

    const user = userEvent.setup();
    await user.click(screen.getByTestId("select-all"));

    // The "Approve & trust" button is offered because all selected items
    // share the same event_type.
    const trust = await screen.findByTestId("bulk-approve-trust");
    expect(trust).toHaveTextContent(/lab_result/);
    await user.click(trust);

    // Confirm dialog mentions auto-approval.
    expect(await screen.findByText(/Trust forever:/)).toBeInTheDocument();

    mockListPending.mockResolvedValueOnce([]);
    await user.click(screen.getByTestId("bulk-confirm-go"));

    await waitFor(() => expect(mockApprovePending).toHaveBeenCalledTimes(2));
    // Only the FIRST call carries the auto-approve flag — storage flips
    // the grant's allowlist on that single call.
    expect(mockApprovePending.mock.calls[0][1]).toBe(true);
    expect(mockApprovePending.mock.calls[1][1]).toBe(false);
  });

  it("approve & trust button is hidden when items have mixed event_types", async () => {
    mockListPending.mockResolvedValueOnce([
      makePending(1, "lab_result"),
      makePending(2, "clinical_note"),
    ]);

    mount();
    await waitFor(() =>
      expect(screen.getByTestId("pending-page")).toBeInTheDocument(),
    );

    const user = userEvent.setup();
    await user.click(screen.getByTestId("select-all"));

    expect(screen.getByTestId("bulk-approve")).toBeInTheDocument();
    // No "trust" affordance when types differ — the §6.1
    // auto_for_event_types path needs a single concrete event_type.
    expect(screen.queryByTestId("bulk-approve-trust")).toBeNull();
  });

  it("reject flow accepts a reason + sends it on each call", async () => {
    mockListPending.mockResolvedValueOnce([
      makePending(1, "lab_result"),
      makePending(2, "lab_result"),
    ]);
    mockRejectPending.mockResolvedValue({ rejectedAtMs: 1 });

    mount();
    await waitFor(() =>
      expect(screen.getByTestId("pending-page")).toBeInTheDocument(),
    );

    const user = userEvent.setup();
    await user.click(screen.getByTestId("select-all"));
    await user.click(screen.getByTestId("bulk-reject"));

    const reasonInput = await screen.findByTestId("bulk-reason");
    await user.type(reasonInput, "duplicate of <ulid>");

    mockListPending.mockResolvedValueOnce([]);
    await user.click(screen.getByTestId("bulk-confirm-go"));

    await waitFor(() => expect(mockRejectPending).toHaveBeenCalledTimes(2));
    expect(mockRejectPending.mock.calls[0][1]).toBe("duplicate of <ulid>");
    expect(mockRejectPending.mock.calls[1][1]).toBe("duplicate of <ulid>");
  });

  it("mid-batch error pauses + offers continue/abort", async () => {
    mockListPending.mockResolvedValueOnce([
      makePending(1, "lab_result"),
      makePending(2, "lab_result"),
      makePending(3, "lab_result"),
    ]);
    // First succeeds; second fails; third never attempted (paused).
    mockApprovePending
      .mockResolvedValueOnce({ committedAtMs: 1 })
      .mockRejectedValueOnce(new Error("storage transient: 503"));

    mount();
    await waitFor(() =>
      expect(screen.getByTestId("pending-page")).toBeInTheDocument(),
    );

    const user = userEvent.setup();
    await user.click(screen.getByTestId("select-all"));
    await user.click(screen.getByTestId("bulk-approve"));
    await user.click(await screen.findByTestId("bulk-confirm-go"));

    // Continue / abort affordances surface after the second call fails.
    await screen.findByTestId("bulk-continue");
    expect(screen.getByTestId("bulk-abort")).toBeInTheDocument();
    // 1 succeeded, 1 failed, 1 remaining (the bullet list, not the button).
    expect(screen.getByText("1 succeeded")).toBeInTheDocument();
    expect(screen.getByText("1 failed")).toBeInTheDocument();
    expect(screen.getByText("1 remaining")).toBeInTheDocument();

    // Continue: third call goes through.
    mockApprovePending.mockResolvedValueOnce({ committedAtMs: 2 });
    mockListPending.mockResolvedValueOnce([]);
    await user.click(screen.getByTestId("bulk-continue"));

    // After continue, the batch finishes; "Done" is shown.
    const done = await screen.findByTestId("bulk-done");
    await user.click(done);
    // Three approve attempts total (one failed).
    expect(mockApprovePending).toHaveBeenCalledTimes(3);
  });
});
