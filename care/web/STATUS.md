# OHD Care — Web App Status

This is the implementation status for `care/web/`. The cross-form
status board is in `care/STATUS.md`.

## Current state — 2026-05-09

| Surface | State | Test coverage |
|---|---|---|
| Roster + per-patient view | Done | smoke (1) |
| Pending queue (bulk approve/reject + trust forever) | Done | PendingPage (6) |
| Operator-side audit log (per-RPC, query-hash canonical) | Done | canonicalQueryHash golden (8) |
| **Two-sided audit panel** (storage AuditQuery + operator-side JOIN) | **Done** | AuditPage (4) |
| **MCP integration — chat panel routing tool calls through Care MCP** | **Done** | ChatPage (5) |
| **Settings → MCP page** (mcpUrl / llmUrl / llmKey / model / no-PHI banner) | **Done** | covered via ChatPage settings seed |

24 vitest cases green; `pnpm typecheck` clean; `pnpm build` clean
(49 modules, 203KiB JS / 14KiB CSS).

## Two-sided audit panel — `pages/AuditPage.tsx`

Per `care/SPEC.md` §7 the panel renders BOTH sides:

- **Storage side**: `OhdcService.AuditQuery` server-streaming RPC,
  wired via `ohdc/client.ts::auditQuery(filter)`. Self-session sees
  all rows; grant tokens are scoped to their own (storage enforces).
- **Operator side**: `ohdc/operatorAudit.ts` rolling localStorage
  buffer, populated by every OHDC dispatch with the canonical
  `query_hash`.
- **JOIN**: by `query_hash` (re-computed from
  `(query_kind, query_params_json)` on storage rows via
  `canonicalQueryHashFromRawJson`). Fallback for non-pending-query
  rows: match by `(action, ts ± 5s)`. Asymmetric rows are flagged
  red — that's the real audit signal.

Filter chips: actor (self / grant), op-kind bucket (read / write /
pending / case_op), time window (1h / 24h / 7d / 30d / all). Refresh
button re-fires the RPC. "Export CSV" downloads the joined view
(matched + asymmetric rows; full 64-char hashes; ULIDs as Crockford).

## MCP integration — `pages/ChatPage.tsx`

Streamable HTTP transport via
`@modelcontextprotocol/sdk@1.18.0`'s
`StreamableHTTPClientTransport`. The MCP endpoint is configured per
operator in **Settings → MCP** and persisted in localStorage
(`ohd-care-mcp-settings`). Build-time defaults from `VITE_MCP_URL`,
`VITE_LLM_URL`, `VITE_LLM_API_KEY`, `VITE_LLM_MODEL`,
`VITE_NO_PHI_TO_EXTERNAL_LLMS`.

Auth: the operator's OIDC bearer (from `ohdc/oidc.ts::loadSession`)
is forwarded as `Authorization: Bearer` to the MCP endpoint. Care
MCP's `OAuthProxy` (`care/mcp/src/ohd_care_mcp/server.py`) validates
upstream.

LLM: OpenAI-compatible chat completions with `tools: [...]`
function-calling. The catalog is `MCPClient.listTools()` mapped to
the OpenAI tool schema in `mcp/llm.ts::toolsForOpenAI`. Local
backends (Ollama, vLLM, llama.cpp) work via the same shape.

Round-trip loop (max 6 LLM rounds per user turn):

1. POST chat completion with `messages + tools`.
2. If `tool_calls` present: for each, parse args. **Write tools**
   (`submit_*` + case mutations per `mcp/llm.ts::CASE_MUTATION_TOOLS`)
   open the **"Submitting to <patient> — confirm?"** modal and block
   the dispatch on the operator's click. Read tools dispatch
   immediately. Tool replies are appended as `role: "tool"`.
3. Loop. Stop when the LLM emits a final `assistant` message with no
   `tool_calls`.

Tool calls render inline in the thread as collapsible
`<details>Used find_relevant_context_for_complaint(...)</details>`
entries (showing args + result JSON / error).

Active patient is mirrored from the URL match (PatientPage's slug)
or the v0 single-grant fallback. Per SPEC §10.6 the LLM can only
change context via `switch_patient(label)`.

PHI banner: when settings flag `noPhiToExternalLlms` is true, the
chat header shows "PHI not sent to external LLMs; tool calls only
stay on this network." Otherwise an amber banner warns "PHI may flow
to <llm_url>". The actual enforcement lives in the MCP server's
`OHD_CARE_NO_PHI_TO_EXTERNAL_LLMS=1` env var; the UI surfaces the
operator's awareness.

## What's still TBD (not blocking C / D)

- **Multi-grant vault wiring inside the chat panel.** Currently the
  active patient comes from the URL or the v0 single-grant fallback;
  when v0.x's vault lands, the chat panel should track the active
  grant via `switch_patient` results from the MCP server.
- **Streaming responses.** Each LLM round is one fetch; for v0 we
  buffer the response. Streaming SSE is a polish pass.
- **Tool-permission policy on the operator side.** Some clinics may
  want to block specific tool names from running at all (eg. no
  `force_close_case` from the chat panel). `mcp/settings.ts` is the
  natural home for this.
- **Audit page tail.** The `AuditQuery.tail=true` mode is wired in
  the proto but the panel currently only does one-shot loads;
  live-updating the panel via the streaming responses is a follow-up.
