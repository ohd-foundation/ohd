// MCP / LLM operator settings — persisted in localStorage.
//
// The Care MCP server can be deployed in two shapes:
//   1. **stdio** — operator-local, paired to a desktop chat app. Not
//      reachable from the browser; not relevant here.
//   2. **Streamable HTTP** — operator-deployed, behind FastMCP's
//      `OAuthProxy` (or naked when `VITE_DEV` and trusted network).
//
// care/web only speaks the Streamable HTTP shape; the URL points at the
// `<mcp_url>/mcp` endpoint that FastMCP exposes. The operator's OIDC
// access token (when present) is forwarded as `Authorization: Bearer`.
//
// LLM endpoint config is OpenAI-compatible chat completions
// (`/v1/chat/completions`). Most clinics either deploy a local Ollama
// or proxy a vetted vendor — Care doesn't pick. The
// `OHD_CARE_NO_PHI_TO_EXTERNAL_LLMS` knob lives on the MCP side; the
// web UI just surfaces the toggle so the operator knows the posture.
//
// Notes:
//  - `localStorage` is per-origin, persists across sessions; we store
//    the LLM API key here only because the alternative is "ask each
//    session" which is hostile to clinical use. Operators who don't
//    want this should set the keys via VITE env vars at build time.
//  - The MCP URL is also the source of truth for FastMCP's session
//    persistence — we leave session-id management to the SDK transport.

const STORAGE_KEY = "ohd-care-mcp-settings";

export interface McpSettings {
  /** e.g. "https://care.clinic.example.com/mcp". Empty = MCP disabled. */
  mcpUrl: string;
  /** OpenAI-compatible chat completions base URL, e.g. "https://api.openai.com/v1". */
  llmUrl: string;
  /** Bearer key for the LLM endpoint. Stored in localStorage; clear with "Sign out". */
  llmApiKey: string;
  /** Model id, e.g. "gpt-4.1-mini" / "claude-3-5-sonnet" / "qwen2.5-coder:7b". */
  llmModel: string;
  /**
   * When true, the operator has confirmed PHI does not flow to the
   * configured LLM endpoint (per the MCP server's
   * `OHD_CARE_NO_PHI_TO_EXTERNAL_LLMS=1` knob). Surfaced as a banner
   * on the chat page.
   */
  noPhiToExternalLlms: boolean;
}

/** Empty defaults — UI prompts the operator to fill them in. */
function emptySettings(): McpSettings {
  const env = (typeof import.meta !== "undefined" ? import.meta.env : undefined) as
    | Record<string, string | undefined>
    | undefined;
  return {
    mcpUrl: env?.VITE_MCP_URL ?? "",
    llmUrl: env?.VITE_LLM_URL ?? "",
    llmApiKey: env?.VITE_LLM_API_KEY ?? "",
    llmModel: env?.VITE_LLM_MODEL ?? "gpt-4.1-mini",
    noPhiToExternalLlms: env?.VITE_NO_PHI_TO_EXTERNAL_LLMS === "1",
  };
}

export function loadMcpSettings(): McpSettings {
  if (typeof window === "undefined") return emptySettings();
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (!raw) return emptySettings();
    const parsed = JSON.parse(raw) as Partial<McpSettings>;
    const defaults = emptySettings();
    return {
      mcpUrl: parsed.mcpUrl ?? defaults.mcpUrl,
      llmUrl: parsed.llmUrl ?? defaults.llmUrl,
      llmApiKey: parsed.llmApiKey ?? defaults.llmApiKey,
      llmModel: parsed.llmModel ?? defaults.llmModel,
      noPhiToExternalLlms:
        parsed.noPhiToExternalLlms ?? defaults.noPhiToExternalLlms,
    };
  } catch {
    return emptySettings();
  }
}

export function saveMcpSettings(s: McpSettings): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(s));
  } catch {
    // ignore quota / private-mode failures; the form will warn the user.
  }
}

export function clearMcpSettings(): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.removeItem(STORAGE_KEY);
  } catch {
    // ignore
  }
}
