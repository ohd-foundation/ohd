// Mount tests for ChatPage.
//
// We mock both the LLM endpoint (`chatComplete`) and the MCP client
// (`buildMCPClientFromSettings` returns a stubbed `MCPClient`). The
// page should:
//   - render the active patient + the PHI banner appropriate to settings,
//   - send the operator's message to the LLM,
//   - dispatch a non-write tool_call to the MCP client and feed the
//     result back into the conversation,
//   - block a write-tool tool_call behind the confirm modal and wait for
//     the operator to click "Confirm".
//
// Settings are seeded via `localStorage` before mount.

import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router-dom";
import { ToastProvider } from "../components/Toast";
import { saveMcpSettings } from "../mcp/settings";
import type { ToolDescriptor } from "../mcp/client";

const mockListTools = vi.fn();
const mockCallTool = vi.fn();
const mockClose = vi.fn();
const mockChatComplete = vi.fn();

// Stand-in MCPClient. We mock `buildMCPClientFromSettings` to hand back
// this stub regardless of the URL.
class StubMCPClient {
  // eslint-disable-next-line @typescript-eslint/no-unused-vars
  constructor(_cfg: unknown) {}
  async connect() { return; }
  listTools() { return mockListTools(); }
  callTool(name: string, args: Record<string, unknown>) {
    return mockCallTool(name, args);
  }
  close() { return mockClose(); }
}

vi.mock("../mcp/client", async () => {
  const actual =
    await vi.importActual<typeof import("../mcp/client")>("../mcp/client");
  return {
    ...actual,
    buildMCPClientFromSettings: () => new StubMCPClient({}),
  };
});

vi.mock("../mcp/llm", async () => {
  const actual = await vi.importActual<typeof import("../mcp/llm")>("../mcp/llm");
  return {
    ...actual,
    chatComplete: (...args: unknown[]) => mockChatComplete(...args),
  };
});

import { ChatPage } from "./ChatPage";

function seedSettings(noPhi = true) {
  saveMcpSettings({
    mcpUrl: "https://care.example.com/mcp",
    llmUrl: "https://llm.example.com/v1",
    llmApiKey: "sk-test",
    llmModel: "test-model",
    noPhiToExternalLlms: noPhi,
  });
}

const TOOLS: ToolDescriptor[] = [
  {
    name: "query_events",
    description: "Read events for the active patient.",
    inputSchema: { type: "object", properties: { event_type: { type: "string" } } },
  },
  {
    name: "submit_clinical_note",
    description: "Submit a clinical note (write tool).",
    inputSchema: {
      type: "object",
      properties: { note_text: { type: "string" }, confirm: { type: "boolean" } },
    },
  },
];

function mount() {
  return render(
    <ToastProvider>
      <MemoryRouter>
        <ChatPage />
      </MemoryRouter>
    </ToastProvider>,
  );
}

describe("ChatPage — LLM + MCP integration", () => {
  beforeEach(() => {
    mockListTools.mockReset();
    mockCallTool.mockReset();
    mockClose.mockReset();
    mockChatComplete.mockReset();
    window.localStorage.clear();
    seedSettings(true);
  });

  it("renders banner + tool catalog count + active-patient placeholder", async () => {
    mockListTools.mockResolvedValueOnce(TOOLS);

    mount();
    await waitFor(() =>
      expect(screen.getByTestId("chat-page")).toBeInTheDocument(),
    );
    // PHI-not-sent banner is on (we set noPhi=true).
    expect(screen.getByTestId("chat-no-phi-banner")).toBeInTheDocument();
    // Tool catalog loaded.
    await waitFor(() =>
      expect(screen.getByText(/2 available/)).toBeInTheDocument(),
    );
  });

  it("a non-write tool_call from the LLM dispatches via MCPClient and renders the result", async () => {
    mockListTools.mockResolvedValueOnce(TOOLS);

    // First LLM round: emit a tool_call.
    mockChatComplete.mockResolvedValueOnce({
      message: {
        role: "assistant",
        tool_calls: [
          {
            id: "call_1",
            type: "function",
            function: {
              name: "query_events",
              arguments: JSON.stringify({ event_type: "std.heart_rate_resting" }),
            },
          },
        ],
      },
      finishReason: "tool_calls",
    });
    // MCP returns the events.
    mockCallTool.mockResolvedValueOnce({
      isError: false,
      text: JSON.stringify({ events: [{ ts: 1, hr: 72 }], active_patient: "alice" }),
      rawContent: [
        { type: "text", text: JSON.stringify({ events: [{ ts: 1, hr: 72 }] }) },
      ],
      json: { events: [{ ts: 1, hr: 72 }] },
    });
    // Second LLM round: final answer with no tool_calls.
    mockChatComplete.mockResolvedValueOnce({
      message: {
        role: "assistant",
        content: "Alice's resting HR is 72 bpm.",
      },
      finishReason: "stop",
    });

    mount();
    await waitFor(() =>
      expect(screen.getByText(/2 available/)).toBeInTheDocument(),
    );

    const user = userEvent.setup();
    await user.type(
      screen.getByTestId("chat-input"),
      "What was her latest HR?",
    );
    await user.click(screen.getByTestId("chat-submit"));

    // LLM is called twice (initial + post-tool-result).
    await waitFor(() => expect(mockChatComplete).toHaveBeenCalledTimes(2));

    // MCP tool was dispatched.
    expect(mockCallTool).toHaveBeenCalledWith("query_events", {
      event_type: "std.heart_rate_resting",
    });

    // The tool call shows up in the thread.
    expect(screen.getByTestId("chat-tool-query_events")).toBeInTheDocument();

    // Final assistant answer renders.
    expect(
      await screen.findByText(/Alice's resting HR is 72 bpm/),
    ).toBeInTheDocument();
  });

  it("a write tool_call surfaces the confirm dialog and gates dispatch on user click", async () => {
    mockListTools.mockResolvedValueOnce(TOOLS);

    // LLM emits a write tool call.
    mockChatComplete.mockResolvedValueOnce({
      message: {
        role: "assistant",
        tool_calls: [
          {
            id: "call_w",
            type: "function",
            function: {
              name: "submit_clinical_note",
              arguments: JSON.stringify({
                note_text: "BP elevated",
                confirm: true,
              }),
            },
          },
        ],
      },
      finishReason: "tool_calls",
    });
    mockCallTool.mockResolvedValueOnce({
      isError: false,
      text: JSON.stringify({ ok: true, ulid: "01HZZZ" }),
      rawContent: [{ type: "text", text: "ok" }],
    });
    mockChatComplete.mockResolvedValueOnce({
      message: { role: "assistant", content: "Note submitted." },
      finishReason: "stop",
    });

    mount();
    await waitFor(() =>
      expect(screen.getByText(/2 available/)).toBeInTheDocument(),
    );

    const user = userEvent.setup();
    await user.type(screen.getByTestId("chat-input"), "Add a clinical note.");
    await user.click(screen.getByTestId("chat-submit"));

    // Confirm modal appears; MCP tool NOT called yet.
    const confirm = await screen.findByTestId("chat-confirm-write");
    expect(confirm).toBeInTheDocument();
    expect(mockCallTool).not.toHaveBeenCalled();

    // Click "Confirm and submit".
    await user.click(screen.getByTestId("chat-confirm-go"));

    await waitFor(() =>
      expect(mockCallTool).toHaveBeenCalledWith("submit_clinical_note", {
        note_text: "BP elevated",
        confirm: true,
      }),
    );

    expect(await screen.findByText(/Note submitted/)).toBeInTheDocument();
  });

  it("declining the confirm dialog feeds an explicit refusal into the conversation", async () => {
    mockListTools.mockResolvedValueOnce(TOOLS);

    mockChatComplete.mockResolvedValueOnce({
      message: {
        role: "assistant",
        tool_calls: [
          {
            id: "call_w",
            type: "function",
            function: {
              name: "submit_clinical_note",
              arguments: JSON.stringify({ note_text: "x" }),
            },
          },
        ],
      },
      finishReason: "tool_calls",
    });
    // After we decline, the LLM follow-up sees the operator_declined
    // message and emits a final apology.
    mockChatComplete.mockResolvedValueOnce({
      message: {
        role: "assistant",
        content: "Understood — note not submitted.",
      },
      finishReason: "stop",
    });

    mount();
    await waitFor(() =>
      expect(screen.getByText(/2 available/)).toBeInTheDocument(),
    );

    const user = userEvent.setup();
    await user.type(screen.getByTestId("chat-input"), "Note please.");
    await user.click(screen.getByTestId("chat-submit"));

    await screen.findByTestId("chat-confirm-write");
    await user.click(screen.getByTestId("chat-confirm-cancel"));

    // MCP tool NOT called.
    expect(mockCallTool).not.toHaveBeenCalled();

    // Final answer renders.
    expect(
      await screen.findByText(/note not submitted/i),
    ).toBeInTheDocument();

    // The next message sent to the LLM included the operator-declined
    // tool reply (so the LLM sees why it should stop).
    const secondCallArgs = mockChatComplete.mock.calls[1][0] as {
      messages: { role: string; content?: string | null }[];
    };
    const toolReply = secondCallArgs.messages.find(
      (m) => m.role === "tool",
    );
    expect(toolReply).toBeDefined();
    expect(toolReply!.content).toContain("operator_declined_confirmation");
  });

  it("renders a 'configure me' empty state when settings are missing", () => {
    window.localStorage.clear();
    saveMcpSettings({
      mcpUrl: "",
      llmUrl: "",
      llmApiKey: "",
      llmModel: "",
      noPhiToExternalLlms: false,
    });
    mount();
    expect(screen.getByTestId("chat-not-configured")).toBeInTheDocument();
  });
});
