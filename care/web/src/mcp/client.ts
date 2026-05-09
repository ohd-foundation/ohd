// MCPClient — thin wrapper around `@modelcontextprotocol/sdk` for care/web.
//
// Care MCP exposes 20 tools per `care/SPEC.md` §10. We connect to
// `<mcp_url>/mcp` over Streamable HTTP, list the tools, and route LLM
// tool calls through the SDK's `callTool` API.
//
// Auth: the same operator OIDC bearer used for OHDC requests is
// forwarded as `Authorization: Bearer` via `requestInit.headers`.
// The Care MCP server's `OAuthProxy` validates this against the
// configured upstream OIDC issuer (per `care/mcp/server.py` and
// `spec/care-auth.md` "Operator authentication into Care").
//
// The SDK exposes `Client` with `listTools()` / `callTool()`. We thinly
// wrap them to:
//   - lazily construct the transport on first call,
//   - surface a clean ToolDescriptor[] (drops the SDK's zod-passthrough
//     wrapper from the chat-side function-calling schema),
//   - centralize error mapping ("MCP not configured" / "MCP unreachable" /
//     "tool returned error").

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

/** A tool, normalized to what the LLM function-calling API expects. */
export interface ToolDescriptor {
  name: string;
  description: string;
  /** JSON-schema for the tool's input. Forwarded directly to the LLM. */
  inputSchema: Record<string, unknown>;
}

/** Result of one `callTool` invocation, normalized for the chat UI. */
export interface ToolCallResult {
  /** True when the MCP server flagged the result as an error (still returned content). */
  isError: boolean;
  /**
   * Stringified content blocks the LLM should see next turn. The MCP
   * spec returns a list of typed blocks (`text` / `image` / …); for
   * v0 we collapse text blocks into a single string and discard the
   * rest. The chat panel renders the full structured view from
   * `rawContent`.
   */
  text: string;
  /** The raw `content` array from the MCP response (for the UI). */
  rawContent: unknown[];
  /** Parsed JSON from `text` (when the tool emitted JSON), or null. */
  json?: unknown;
}

export interface MCPClientConfig {
  /** Full URL to the MCP endpoint, e.g. https://care.clinic/mcp */
  url: string;
  /** Optional bearer token (operator OIDC access token). */
  bearer?: string | null;
  /** Hook for testing — overrides the SDK's fetch. */
  fetchFn?: typeof fetch;
}

/**
 * Stateful MCP client. Lazily connects on first call; safe to call
 * `listTools` / `callTool` repeatedly. Disposed via `close()`.
 */
export class MCPClient {
  private cfg: MCPClientConfig;
  private client: Client | null = null;
  private transport: StreamableHTTPClientTransport | null = null;
  private connected: Promise<void> | null = null;

  constructor(cfg: MCPClientConfig) {
    this.cfg = cfg;
  }

  /** Idempotent — first call opens the transport; later calls are no-ops. */
  async connect(): Promise<void> {
    if (this.connected) return this.connected;
    this.connected = (async () => {
      const url = new URL(this.cfg.url);
      const requestInit: RequestInit = {};
      if (this.cfg.bearer) {
        requestInit.headers = { Authorization: `Bearer ${this.cfg.bearer}` };
      }
      this.transport = new StreamableHTTPClientTransport(url, {
        requestInit,
        fetch: this.cfg.fetchFn,
      });
      this.client = new Client(
        { name: "ohd-care-web", version: "0.1.0" },
        { capabilities: {} },
      );
      await this.client.connect(this.transport);
    })();
    return this.connected;
  }

  /** List the MCP server's tool catalog. */
  async listTools(): Promise<ToolDescriptor[]> {
    await this.connect();
    if (!this.client) throw new Error("MCP client not connected");
    const resp = await this.client.listTools();
    return (resp.tools ?? []).map((t) => ({
      name: t.name,
      description: t.description ?? "",
      // The SDK returns `inputSchema` as a zod-passthrough object; cast
      // to a plain JSON-Schema object for the LLM. Empty objects are
      // valid (tools that take no args).
      inputSchema: (t.inputSchema as Record<string, unknown>) ?? { type: "object" },
    }));
  }

  /** Invoke one tool. Returns the normalized result. */
  async callTool(name: string, args: Record<string, unknown>): Promise<ToolCallResult> {
    await this.connect();
    if (!this.client) throw new Error("MCP client not connected");
    const resp = await this.client.callTool({ name, arguments: args });
    const content = (resp.content as unknown[] | undefined) ?? [];
    // Collapse text blocks. The MCP spec's text-block shape:
    //   { type: "text", text: "..." }
    let text = "";
    for (const block of content) {
      const b = block as { type?: string; text?: string };
      if (b.type === "text" && typeof b.text === "string") {
        text += b.text;
      }
    }
    let json: unknown | undefined;
    if (text.trim().length > 0) {
      try {
        json = JSON.parse(text);
      } catch {
        // Not JSON; that's fine — leave undefined.
      }
    }
    return {
      isError: Boolean(resp.isError),
      text,
      rawContent: content,
      json,
    };
  }

  /** Close the transport. Safe to call repeatedly. */
  async close(): Promise<void> {
    try {
      if (this.client) await this.client.close();
    } catch {
      // ignore
    }
    this.client = null;
    this.transport = null;
    this.connected = null;
  }
}

/**
 * Convenience: build an MCPClient from the runtime settings + the
 * operator's OIDC session. Returns null when the URL isn't configured.
 */
export function buildMCPClientFromSettings(opts: {
  mcpUrl: string;
  bearer?: string | null;
}): MCPClient | null {
  if (!opts.mcpUrl) return null;
  return new MCPClient({ url: opts.mcpUrl, bearer: opts.bearer ?? null });
}
