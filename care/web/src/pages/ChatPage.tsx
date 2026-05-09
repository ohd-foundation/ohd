// ChatPage — operator-facing LLM chat panel routed through Care MCP.
//
// Per `care/SPEC.md` §10 the operator's LLM tools live in the Care MCP
// server; this page surfaces them as a chat UI:
//
//   - Top bar: active patient indicator (mirrors PatientPage's header).
//     The chat is scoped to the active patient grant — switching patients
//     mid-thread requires `switch_patient(...)` from the LLM (per
//     SPEC §10.6 "switch_patient is the only tool that changes active
//     context").
//   - Middle: scrollable thread of user / assistant messages plus
//     collapsible "Used <tool>(...)" annotations for every tool round
//     trip.
//   - Bottom: textarea + submit. Cmd/Ctrl + Enter submits.
//   - On submit: post to the configured OpenAI-compatible chat endpoint
//     with the MCP tool catalog as `tools`. Loop:
//       - LLM emits assistant message with tool_calls →
//         per write tool, prompt the operator with
//         "Submitting to <patient> — confirm?".
//         On confirm, dispatch to `MCPClient.callTool`. On cancel,
//         feed an explicit refusal back into the conversation.
//       - Tool results go into the thread as `role: "tool"` messages.
//       - Loop until the LLM returns no tool_calls.
//
// Settings come from `mcp/settings.ts` (localStorage). When the MCP /
// LLM URL isn't configured, the page renders a "go to Settings" CTA.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Link, useMatch } from "react-router-dom";
import { getPatientBySlug } from "../mock/store";
import { useToast } from "../components/Toast";
import { loadSession } from "../ohdc/oidc";
import {
  buildMCPClientFromSettings,
  type MCPClient,
  type ToolDescriptor,
} from "../mcp/client";
import {
  chatComplete,
  isWriteTool,
  parseToolArgs,
  type ChatRole,
  type ChatToolCall,
} from "../mcp/llm";
import { loadMcpSettings, type McpSettings } from "../mcp/settings";

// --- Thread model ----------------------------------------------------------

interface ThreadEntryUser {
  kind: "user";
  text: string;
  ts: number;
}
interface ThreadEntryAssistant {
  kind: "assistant";
  text: string;
  ts: number;
}
interface ThreadEntryTool {
  kind: "tool";
  toolName: string;
  args: Record<string, unknown>;
  result?: { isError: boolean; text: string; rawJson?: unknown };
  ts: number;
  /** True when this entry is awaiting operator confirmation. */
  awaitingConfirm?: boolean;
  /** True when the operator declined the confirmation. */
  declined?: boolean;
}
interface ThreadEntrySystem {
  kind: "system";
  text: string;
  ts: number;
  variant?: "info" | "error";
}

type ThreadEntry =
  | ThreadEntryUser
  | ThreadEntryAssistant
  | ThreadEntryTool
  | ThreadEntrySystem;

// --- Props -----------------------------------------------------------------

const SYSTEM_PROMPT = (
  activeLabel: string | null,
  noPhi: boolean,
): string =>
  [
    "You are an operator-facing clinical assistant routed through OHD Care MCP.",
    activeLabel
      ? `The active patient is ${activeLabel}. All read/write tools scope to this patient until you call switch_patient(...).`
      : "No active patient is selected. Use list_patients() and switch_patient(label) to set one.",
    "Per care/SPEC.md §10.6: switch_patient is the ONLY way to change active context; never assume.",
    "Write tools (submit_*) require confirm=true and trigger an explicit operator confirmation in the UI before they reach storage.",
    noPhi
      ? "PHI does not flow to this LLM endpoint per the deployment's OHD_CARE_NO_PHI_TO_EXTERNAL_LLMS=1 posture; tool results stay on the operator's network."
      : "PHI may flow to this LLM endpoint per the deployment posture; use only with operator's awareness.",
  ].join(" ");

// --- Component -------------------------------------------------------------

export function ChatPage() {
  const toast = useToast();
  const settings = useMemo(() => loadMcpSettings(), []);
  const session = loadSession();
  const operatorBearer = session?.accessToken ?? null;

  // Active patient — derived from the URL match like AppShell does, plus a
  // fallback to "the only patient v0 has" for the single-grant case.
  const patientMatch = useMatch("/patient/:label/*");
  const slug = patientMatch?.params.label;
  const patientFromUrl = slug ? getPatientBySlug(slug) : undefined;
  const patientFallback = useMemo(() => getPatientBySlug("patient"), []);
  const patient = patientFromUrl ?? patientFallback;
  const patientLabel = patient?.label ?? null;

  // MCP client — lazy; rebuilt when the URL changes.
  const mcpRef = useRef<MCPClient | null>(null);
  const [tools, setTools] = useState<ToolDescriptor[] | null>(null);
  const [toolsError, setToolsError] = useState<string | null>(null);

  useEffect(() => {
    if (!settings.mcpUrl) return;
    const client = buildMCPClientFromSettings({
      mcpUrl: settings.mcpUrl,
      bearer: operatorBearer,
    });
    mcpRef.current = client;
    if (!client) return;
    let cancelled = false;
    void (async () => {
      try {
        const t = await client.listTools();
        if (!cancelled) {
          setTools(t);
          setToolsError(null);
        }
      } catch (err) {
        if (!cancelled) {
          setToolsError((err as Error).message ?? String(err));
        }
      }
    })();
    return () => {
      cancelled = true;
      void client.close();
    };
  }, [settings.mcpUrl, operatorBearer]);

  // Conversation state — what the user sees AND what we send to the LLM.
  const [thread, setThread] = useState<ThreadEntry[]>([]);
  // The OpenAI-shaped message history; mirrors `thread` with the system
  // prompt and tool-call/tool-result wiring.
  const [messages, setMessages] = useState<ChatRole[]>([]);
  const [input, setInput] = useState<string>("");
  const [busy, setBusy] = useState<boolean>(false);
  const [pendingConfirm, setPendingConfirm] = useState<{
    toolCall: ChatToolCall;
    args: Record<string, unknown>;
  } | null>(null);

  // Scroll to bottom on every new entry.
  const threadEndRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    // jsdom (used in tests) doesn't implement scrollIntoView; guard it.
    threadEndRef.current?.scrollIntoView?.({ behavior: "smooth" });
  }, [thread.length]);

  // Initialize messages with the system prompt once we know the patient.
  useEffect(() => {
    if (messages.length > 0) return;
    setMessages([
      {
        role: "system",
        content: SYSTEM_PROMPT(patientLabel, settings.noPhiToExternalLlms),
      },
    ]);
    // Only on first mount — switching patients mid-thread should NOT
    // wipe the system prompt; the LLM gets a system reminder via
    // `switch_patient` orientation. Per SPEC §10.6.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // --- LLM round-trip loop -------------------------------------------------

  /**
   * Dispatch one tool call. Returns the `role:"tool"` ChatRole that should
   * be appended to the LLM message history (so we don't depend on async
   * state propagation through the ref to feed the next LLM round).
   */
  const dispatchTool = useCallback(
    async (
      toolCall: ChatToolCall,
      args: Record<string, unknown>,
    ): Promise<ChatRole> => {
      const client = mcpRef.current;
      if (!client) {
        appendThread({
          kind: "system",
          text: "MCP client not available; configure it in Settings → MCP.",
          ts: Date.now(),
          variant: "error",
        });
        return {
          role: "tool",
          tool_call_id: toolCall.id,
          name: toolCall.function.name,
          content: JSON.stringify({ error: "mcp_client_not_configured" }),
        };
      }
      const toolEntry: ThreadEntryTool = {
        kind: "tool",
        toolName: toolCall.function.name,
        args,
        ts: Date.now(),
      };
      appendThread(toolEntry);
      try {
        const result = await client.callTool(toolCall.function.name, args);
        replaceTool(toolEntry.ts, {
          ...toolEntry,
          result: {
            isError: result.isError,
            text: result.text,
            rawJson: result.json,
          },
        });
        return {
          role: "tool",
          tool_call_id: toolCall.id,
          name: toolCall.function.name,
          content: result.text || JSON.stringify(result.rawContent),
        };
      } catch (err) {
        const msg = (err as Error).message ?? String(err);
        replaceTool(toolEntry.ts, {
          ...toolEntry,
          result: { isError: true, text: msg },
        });
        return {
          role: "tool",
          tool_call_id: toolCall.id,
          name: toolCall.function.name,
          content: JSON.stringify({ error: msg }),
        };
      }
    },
    [],
  );

  const declineTool = useCallback(
    (toolCall: ChatToolCall, args: Record<string, unknown>): ChatRole => {
      appendThread({
        kind: "tool",
        toolName: toolCall.function.name,
        args,
        ts: Date.now(),
        declined: true,
        result: {
          isError: true,
          text: "Operator declined to confirm this submission.",
        },
      });
      return {
        role: "tool",
        tool_call_id: toolCall.id,
        name: toolCall.function.name,
        content: JSON.stringify({
          error: "operator_declined_confirmation",
          hint: "User refused to authorize the write — do not retry without explicit confirmation in the next turn.",
        }),
      };
    },
    [],
  );

  const runTurn = useCallback(
    async (history: ChatRole[]) => {
      if (!settings.llmUrl || !settings.llmApiKey) {
        appendThread({
          kind: "system",
          text: "LLM endpoint not configured; configure it in Settings → MCP.",
          ts: Date.now(),
          variant: "error",
        });
        return;
      }
      setBusy(true);
      try {
        // Iterate up to 6 LLM round-trips per user turn — enough for the
        // visit-prep / context-fetch tool chains we expect; finite to
        // bound runaway loops.
        let history2 = history;
        for (let i = 0; i < 6; i++) {
          const resp = await chatComplete({
            llmUrl: settings.llmUrl,
            apiKey: settings.llmApiKey,
            model: settings.llmModel,
            messages: history2,
            tools: tools ?? [],
          });
          history2 = [...history2, resp.message];
          setMessages(history2);

          if (resp.message.tool_calls && resp.message.tool_calls.length > 0) {
            // Dispatch each tool_call serially. Write tools require
            // operator confirmation; everything else proceeds. Each
            // dispatch returns the `role:"tool"` ChatRole; we append it
            // to BOTH the local history2 (drives the next LLM round)
            // and the React `messages` state (so the UI's downstream
            // re-renders see the same history).
            const toolReplies: ChatRole[] = [];
            for (const tc of resp.message.tool_calls) {
              const args = parseToolArgs(tc.function.arguments);
              let reply: ChatRole;
              if (isWriteTool(tc.function.name)) {
                // Park the tool call in a confirm dialog. Resolve via
                // setPendingConfirm — handled on user click.
                const result = await new Promise<"confirmed" | "declined">(
                  (resolve) => {
                    setPendingConfirm({ toolCall: tc, args });
                    pendingResolverRef.current = resolve;
                  },
                );
                setPendingConfirm(null);
                pendingResolverRef.current = null;
                if (result === "confirmed") {
                  reply = await dispatchTool(tc, args);
                } else {
                  reply = declineTool(tc, args);
                }
              } else {
                reply = await dispatchTool(tc, args);
              }
              toolReplies.push(reply);
            }
            history2 = [...history2, ...toolReplies];
            appendMessages(toolReplies);
            continue;
          }

          // No tool_calls — final assistant message. Render and stop.
          const assistantText =
            typeof resp.message.content === "string" ? resp.message.content : "";
          if (assistantText) {
            appendThread({
              kind: "assistant",
              text: assistantText,
              ts: Date.now(),
            });
          }
          return;
        }
        appendThread({
          kind: "system",
          text: "LLM tool-call loop hit its 6-round budget without a final answer.",
          ts: Date.now(),
          variant: "error",
        });
      } catch (err) {
        appendThread({
          kind: "system",
          text: `LLM error: ${(err as Error).message ?? String(err)}`,
          ts: Date.now(),
          variant: "error",
        });
      } finally {
        setBusy(false);
      }
    },
    [settings, tools, dispatchTool, declineTool],
  );

  // --- Helpers (need refs to avoid stale closures inside the run loop) -----

  const messagesRef = useRef(messages);
  useEffect(() => {
    messagesRef.current = messages;
  }, [messages]);

  const pendingResolverRef = useRef<((v: "confirmed" | "declined") => void) | null>(null);

  const appendThread = useCallback((e: ThreadEntry) => {
    setThread((prev) => [...prev, e]);
  }, []);

  const replaceTool = useCallback((ts: number, next: ThreadEntryTool) => {
    setThread((prev) =>
      prev.map((e) => (e.kind === "tool" && e.ts === ts ? next : e)),
    );
  }, []);

  const appendMessages = useCallback((msgs: ChatRole[]) => {
    setMessages((prev) => [...prev, ...msgs]);
  }, []);

  // --- Submit --------------------------------------------------------------

  const onSubmit = useCallback(async () => {
    const text = input.trim();
    if (!text) return;
    setInput("");
    appendThread({ kind: "user", text, ts: Date.now() });
    const next = [...messagesRef.current, { role: "user" as const, content: text }];
    setMessages(next);
    await runTurn(next);
  }, [input, runTurn, appendThread]);

  const onConfirmWrite = useCallback(() => {
    pendingResolverRef.current?.("confirmed");
  }, []);
  const onCancelWrite = useCallback(() => {
    pendingResolverRef.current?.("declined");
    toast.show("Submission declined");
  }, [toast]);

  // --- Render --------------------------------------------------------------

  const configMissing = !settings.mcpUrl || !settings.llmUrl || !settings.llmApiKey;

  if (configMissing) {
    return (
      <div className="empty" data-testid="chat-not-configured">
        <h3>Chat is not configured</h3>
        <p>
          Set the MCP endpoint and the LLM endpoint under{" "}
          <Link to="/settings/mcp">Settings → MCP</Link>.
        </p>
      </div>
    );
  }

  return (
    <div className="chat-page" data-testid="chat-page">
      <header className="chat-header">
        <h2>Chat</h2>
        <div className="muted" aria-live="polite">
          {patient ? (
            <>
              <span>Active patient:</span>{" "}
              <strong data-testid="chat-active-patient">{patient.label}</strong>
            </>
          ) : (
            <span>No active patient</span>
          )}
        </div>
      </header>

      {settings.noPhiToExternalLlms ? (
        <div className="chat-banner chat-banner-info" data-testid="chat-no-phi-banner">
          PHI not sent to external LLMs; tool calls only stay on this network.
        </div>
      ) : (
        <div className="chat-banner chat-banner-warn" data-testid="chat-phi-banner">
          PHI may flow to <code>{settings.llmUrl}</code> — verify the deployment posture
          before discussing patient data.
        </div>
      )}

      {toolsError && (
        <div className="chat-banner chat-banner-warn">
          MCP catalog load failed: <code>{toolsError}</code>
        </div>
      )}

      <div className="chat-thread" data-testid="chat-thread">
        {thread.length === 0 && (
          <div className="muted" style={{ fontSize: 13, padding: 16 }}>
            Ask a question — the assistant will use Care MCP tools (
            {tools ? `${tools.length} available` : "loading…"}) scoped to{" "}
            <strong>{patientLabel ?? "no patient"}</strong>.
          </div>
        )}
        {thread.map((e, i) => (
          <ThreadItem key={`${e.ts}-${i}`} entry={e} />
        ))}
        <div ref={threadEndRef} />
      </div>

      <div className="chat-input-row">
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          placeholder="Ask the assistant about this patient..."
          onKeyDown={(e) => {
            if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
              e.preventDefault();
              void onSubmit();
            }
          }}
          data-testid="chat-input"
          disabled={busy || pendingConfirm !== null}
        />
        <button
          type="button"
          className="btn btn-primary"
          onClick={() => void onSubmit()}
          disabled={busy || pendingConfirm !== null || input.trim().length === 0}
          data-testid="chat-submit"
        >
          {busy ? "Working…" : "Send"}
        </button>
      </div>

      {pendingConfirm && (
        <ConfirmWriteDialog
          patientLabel={patientLabel}
          toolCall={pendingConfirm.toolCall}
          args={pendingConfirm.args}
          settings={settings}
          onConfirm={onConfirmWrite}
          onCancel={onCancelWrite}
        />
      )}
    </div>
  );
}

// --- Subcomponents ---------------------------------------------------------

function ThreadItem({ entry }: { entry: ThreadEntry }) {
  if (entry.kind === "user") {
    return (
      <div className="chat-msg chat-msg-user" data-testid="chat-msg-user">
        <div>{entry.text}</div>
      </div>
    );
  }
  if (entry.kind === "assistant") {
    return (
      <div className="chat-msg chat-msg-assistant" data-testid="chat-msg-assistant">
        <div>{entry.text}</div>
      </div>
    );
  }
  if (entry.kind === "system") {
    return (
      <div
        className="chat-msg chat-msg-tool"
        data-testid="chat-msg-system"
        style={
          entry.variant === "error"
            ? { borderColor: "var(--color-accent)" }
            : undefined
        }
      >
        <div>{entry.text}</div>
      </div>
    );
  }
  return <ToolCallDisplay entry={entry} />;
}

function ToolCallDisplay({ entry }: { entry: ThreadEntryTool }) {
  const argsPreview = JSON.stringify(entry.args);
  const summary = `Used ${entry.toolName}(${argsPreview.length > 80 ? "…" : argsPreview.slice(1, -1)})`;
  return (
    <details
      className="chat-msg chat-msg-tool"
      data-testid={`chat-tool-${entry.toolName}`}
      data-declined={entry.declined ? "true" : "false"}
      data-error={entry.result?.isError ? "true" : "false"}
    >
      <summary>{summary}</summary>
      <div className="mono" style={{ fontSize: 11, marginTop: 6 }}>
        <strong>args:</strong> {JSON.stringify(entry.args, null, 2)}
      </div>
      {entry.declined && (
        <div className="mono" style={{ color: "var(--color-accent)", marginTop: 6 }}>
          DECLINED — operator did not confirm.
        </div>
      )}
      {entry.result && (
        <div className="mono" style={{ fontSize: 11, marginTop: 6 }}>
          <strong>{entry.result.isError ? "error:" : "result:"}</strong>{" "}
          {entry.result.text || "(empty)"}
        </div>
      )}
    </details>
  );
}

function ConfirmWriteDialog({
  patientLabel,
  toolCall,
  args,
  settings,
  onConfirm,
  onCancel,
}: {
  patientLabel: string | null;
  toolCall: ChatToolCall;
  args: Record<string, unknown>;
  settings: McpSettings;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="modal-backdrop" data-testid="chat-confirm-write">
      <div className="chat-confirm-modal" role="dialog" aria-labelledby="chat-confirm-title">
        <h3 id="chat-confirm-title" style={{ marginTop: 0 }}>
          Submitting to <strong>{patientLabel ?? "no active patient"}</strong> — confirm?
        </h3>
        <p className="muted" style={{ fontSize: 13 }}>
          The assistant wants to call{" "}
          <code>{toolCall.function.name}</code> on this patient's record.
          Per <code>care/SPEC.md</code> §10.6 every write tool requires
          your explicit confirmation before reaching storage.
        </p>
        <pre
          className="mono"
          style={{ fontSize: 12, background: "var(--color-surface)", padding: 8, borderRadius: 4 }}
        >
{JSON.stringify(args, null, 2)}
        </pre>
        <div
          className="muted"
          style={{ fontSize: 11, fontFamily: "var(--font-mono)", marginBottom: 8 }}
        >
          MCP: <code>{settings.mcpUrl}</code>
          {settings.noPhiToExternalLlms && <> · <strong>NO_PHI_TO_EXTERNAL_LLMS</strong></>}
        </div>
        <div className="modal-actions">
          <button
            type="button"
            className="btn btn-ghost"
            onClick={onCancel}
            data-testid="chat-confirm-cancel"
          >
            Cancel
          </button>
          <button
            type="button"
            className="btn btn-primary"
            onClick={onConfirm}
            data-testid="chat-confirm-go"
          >
            Confirm and submit
          </button>
        </div>
      </div>
    </div>
  );
}
