// Tiny OpenAI-compatible chat-completions client.
//
// We deliberately don't pull in `openai` or `@anthropic-ai/sdk` —
// chat-completions is a single fetch call and adding a transitive dep
// for it doubles the bundle size. This module is also where we
// normalize the function-calling shape across vendors:
//
//   - OpenAI (`tools: [{type: "function", function: {name, parameters,
//     description}}]`).
//   - Anthropic Messages API exposes a parallel shape via
//     `tools: [{name, description, input_schema}]` — out of scope for
//     v0; the Care MCP doc recommends a thin proxy that maps Anthropic
//     replies back to OpenAI shape.
//   - Local backends (vLLM, Ollama, llama.cpp's openai-compat server)
//     all support the OpenAI tool-calling schema.
//
// The chat panel calls `chatComplete(...)` once per turn. When the
// response carries `tool_calls`, the chat panel routes them through
// `MCPClient.callTool` (with operator confirmation for write tools)
// and feeds the results back as `role: "tool"` messages on the next
// turn. The loop terminates when the model returns a final assistant
// message with no tool_calls.

import type { ToolDescriptor } from "./client";

export interface ChatRole {
  role: "system" | "user" | "assistant" | "tool";
  /** For `assistant` messages with no `tool_calls` and `user` / `system` / `tool`. */
  content?: string | null;
  /** For `assistant`-side tool calls. */
  tool_calls?: ChatToolCall[];
  /** For `tool` results — the id of the tool_call we're answering. */
  tool_call_id?: string;
  /** For `tool` results — the tool name. */
  name?: string;
}

export interface ChatToolCall {
  id: string;
  type: "function";
  function: {
    name: string;
    /** JSON-encoded args. The model emits a string we re-parse before dispatch. */
    arguments: string;
  };
}

export interface ChatCompleteOpts {
  /** Endpoint base URL, e.g. "https://api.openai.com/v1". `/chat/completions` is appended. */
  llmUrl: string;
  /** Bearer token for the LLM endpoint. */
  apiKey: string;
  /** Model id. */
  model: string;
  /** Conversation so far. */
  messages: ChatRole[];
  /** MCP tools, mapped to the OpenAI function-calling schema. */
  tools: ToolDescriptor[];
  /** Forced tool-choice ("none" / "auto" / explicit name). Defaults to "auto". */
  toolChoice?: "auto" | "none";
  /** Hook for tests. */
  fetchFn?: typeof fetch;
}

export interface ChatCompleteResult {
  /** The chosen `assistant` message. May contain `tool_calls`. */
  message: ChatRole;
  /** "stop" | "tool_calls" | "length" | "content_filter" | string. */
  finishReason: string;
}

/** Map our `ToolDescriptor` shape to OpenAI's function tool schema. */
export function toolsForOpenAI(tools: ToolDescriptor[]): unknown[] {
  return tools.map((t) => ({
    type: "function",
    function: {
      name: t.name,
      description: t.description,
      parameters:
        t.inputSchema && Object.keys(t.inputSchema).length > 0
          ? t.inputSchema
          : { type: "object", properties: {} },
    },
  }));
}

/** Send one round-trip to the LLM endpoint. */
export async function chatComplete(opts: ChatCompleteOpts): Promise<ChatCompleteResult> {
  const f = opts.fetchFn ?? fetch;
  const url = opts.llmUrl.replace(/\/+$/, "") + "/chat/completions";
  const body: Record<string, unknown> = {
    model: opts.model,
    messages: opts.messages,
    tool_choice: opts.toolChoice ?? "auto",
  };
  if (opts.tools.length > 0) {
    body.tools = toolsForOpenAI(opts.tools);
  }
  const resp = await f(url, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      authorization: `Bearer ${opts.apiKey}`,
    },
    body: JSON.stringify(body),
  });
  if (!resp.ok) {
    const text = await resp.text().catch(() => "");
    throw new Error(`LLM endpoint ${resp.status}: ${text || resp.statusText}`);
  }
  const json = (await resp.json()) as {
    choices?: { message?: ChatRole; finish_reason?: string }[];
  };
  const choice = json.choices?.[0];
  if (!choice?.message) {
    throw new Error("LLM endpoint returned no choices[0].message");
  }
  return {
    message: choice.message,
    finishReason: choice.finish_reason ?? "stop",
  };
}

/**
 * Parse a `tool_calls[i].function.arguments` JSON string. Returns `{}`
 * on failure — better to call the tool with empty args and let it
 * surface a validation error than to crash the chat loop.
 */
export function parseToolArgs(s: string | undefined | null): Record<string, unknown> {
  if (!s) return {};
  try {
    const v = JSON.parse(s);
    if (v && typeof v === "object" && !Array.isArray(v)) {
      return v as Record<string, unknown>;
    }
    return {};
  } catch {
    return {};
  }
}

/** Care MCP's write-tool prefix per `care/SPEC.md` §10.3. UI uses this to gate confirms. */
export const WRITE_TOOL_PREFIX = "submit_";

/**
 * Tool names that mutate patient context but aren't `submit_*`. The
 * chat panel still routes them without an extra confirmation but
 * surfaces them as "write-ish" so the operator can see what's
 * happening. Per `care/SPEC.md` §10.5.
 */
export const CASE_MUTATION_TOOLS = new Set([
  "open_case",
  "close_case",
  "force_close_case",
  "issue_retrospective_grant",
]);

export function isWriteTool(name: string): boolean {
  return name.startsWith(WRITE_TOOL_PREFIX) || CASE_MUTATION_TOOLS.has(name);
}
