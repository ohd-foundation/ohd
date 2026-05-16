// Typed API client for cord-server. Base path is same-origin; the Vite dev
// server proxies /v1 and /healthz to the local backend.

// ---- Types -----------------------------------------------------------------

export interface User {
  cord_user_ulid: string;
  display_label: string;
  created_at: string;
  [k: string]: unknown;
}

export interface Policy {
  allow_user_keys: boolean;
  allow_custom_relay: boolean;
  default_relay: string;
  default_model_provider: string;
}

export interface Me {
  user: User;
  policy: Policy;
}

export interface AuthProvider {
  id: string;
  issuer: string;
}

// A connected data store. The backend calls this a "source" on the wire
// (URLs `/v1/sources*`, JSON fields unchanged); the UI labels it a
// "Connection". `Connection` is the name used throughout the UI code.
export interface Connection {
  id: string;
  label: string;
  kind: string;
  endpoint: string;
  status: string;
  scope_json: string;
  created_at: string;
  last_ok_at: string | null;
  [k: string]: unknown;
}

// Wire-compatible alias kept for clarity at the API boundary.
export type Source = Connection;

export interface SystemProvider {
  id: string;
  kind: string;
  models: string[];
  has_key: boolean;
}

export interface ByoKey {
  id: string;
  provider_kind: string;
  label: string;
  created_at: string;
}

export interface ModelsInfo {
  system_providers: SystemProvider[];
  default_provider: string;
  allow_user_keys: boolean;
  byo_keys: ByoKey[];
}

export interface Chat {
  id: string;
  source_id: string;
  model: string;
  title: string | null;
  created_at: string;
  updated_at: string;
}

export type Role = "user" | "assistant" | "system";

export interface ChatMessage {
  id: string;
  role: Role;
  content: string;
  created_at: string;
}

export interface ChatDetail {
  chat: Chat;
  messages: ChatMessage[];
}

// SSE event shapes emitted by POST /v1/chats/:id/messages.
export type StreamEvent =
  | { type: "text"; delta: string }
  | { type: "tool"; name: string }
  | { type: "done" }
  | { type: "error"; message: string };

// ---- Error -----------------------------------------------------------------

export class ApiError extends Error {
  status: number;
  constructor(status: number, message: string) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

// ---- Core fetch ------------------------------------------------------------

async function request<T>(
  method: string,
  path: string,
  body?: unknown,
): Promise<T> {
  const init: RequestInit = {
    method,
    credentials: "include",
    headers: {},
  };
  if (body !== undefined) {
    init.headers = { "Content-Type": "application/json" };
    init.body = JSON.stringify(body);
  }
  const res = await fetch(path, init);
  if (!res.ok) {
    let message = `${res.status} ${res.statusText}`;
    try {
      const data = await res.json();
      if (data && typeof data.error === "string") message = data.error;
    } catch {
      // non-JSON error body; keep the status text
    }
    throw new ApiError(res.status, message);
  }
  if (res.status === 204) return undefined as T;
  return (await res.json()) as T;
}

// ---- Auth ------------------------------------------------------------------

export const api = {
  authProviders(): Promise<{ providers: AuthProvider[] }> {
    return request("GET", "/v1/auth/providers");
  },

  // The browser must NAVIGATE here, not fetch it.
  authStartUrl(providerId: string): string {
    return `/v1/auth/start?provider=${encodeURIComponent(providerId)}`;
  },

  logout(): Promise<{ ok: boolean }> {
    return request("POST", "/v1/auth/logout");
  },

  me(): Promise<Me> {
    return request("GET", "/v1/me");
  },

  // ---- Connections ---------------------------------------------------------
  // The wire still calls these "sources" (`/v1/sources*`, `{sources:[...]}`,
  // `source_id`); the UI labels them "Connections". URLs/JSON are unchanged.

  connections(): Promise<{ sources: Connection[] }> {
    return request("GET", "/v1/sources");
  },

  addConnection(
    body:
      | { label?: string; link: string }
      | { label?: string; endpoint: string; token: string; pin?: string },
  ): Promise<{ source: Connection }> {
    return request("POST", "/v1/sources/connect", body);
  },

  connection(id: string): Promise<{ source: Connection }> {
    return request("GET", `/v1/sources/${encodeURIComponent(id)}`);
  },

  refreshConnection(id: string): Promise<{ source: Connection }> {
    return request("POST", `/v1/sources/${encodeURIComponent(id)}/refresh`);
  },

  deleteConnection(id: string): Promise<{ ok: boolean }> {
    return request("DELETE", `/v1/sources/${encodeURIComponent(id)}`);
  },

  // ---- Models --------------------------------------------------------------

  models(): Promise<ModelsInfo> {
    return request("GET", "/v1/models");
  },

  addByoKey(body: {
    provider_kind: "anthropic" | "gemini" | "openai";
    label: string;
    api_key: string;
  }): Promise<{ key: ByoKey }> {
    return request("POST", "/v1/models/byo", body);
  },

  deleteByoKey(id: string): Promise<{ ok: boolean }> {
    return request("DELETE", `/v1/models/byo/${encodeURIComponent(id)}`);
  },

  // ---- Chats ---------------------------------------------------------------

  chats(): Promise<{ chats: Chat[] }> {
    return request("GET", "/v1/chats");
  },

  createChat(body: { source_id: string; model?: string }): Promise<{
    chat: Chat;
  }> {
    return request("POST", "/v1/chats", body);
  },

  chat(id: string): Promise<ChatDetail> {
    return request("GET", `/v1/chats/${encodeURIComponent(id)}`);
  },

  deleteChat(id: string): Promise<{ ok: boolean }> {
    return request("DELETE", `/v1/chats/${encodeURIComponent(id)}`);
  },
};

// ---- Chat streaming --------------------------------------------------------

// Sends a message and consumes the SSE response, invoking onEvent per event.
// Throws ApiError on a non-2xx response (e.g. 501 while the agent is not yet
// available) so callers can show a friendly notice.
export async function streamMessage(
  chatId: string,
  message: string,
  onEvent: (ev: StreamEvent) => void,
  signal?: AbortSignal,
): Promise<void> {
  const res = await fetch(
    `/v1/chats/${encodeURIComponent(chatId)}/messages`,
    {
      method: "POST",
      credentials: "include",
      headers: { "Content-Type": "application/json", Accept: "text/event-stream" },
      body: JSON.stringify({ message }),
      signal,
    },
  );

  if (!res.ok) {
    let msg = `${res.status} ${res.statusText}`;
    try {
      const data = await res.json();
      if (data && typeof data.error === "string") msg = data.error;
    } catch {
      // ignore
    }
    throw new ApiError(res.status, msg);
  }

  if (!res.body) throw new ApiError(0, "empty response body");

  const reader = res.body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  // Parse text/event-stream: events separated by a blank line; within an
  // event, lines prefixed `event:` / `data:`. We only need the `data:` JSON.
  const flush = (raw: string) => {
    const dataLines: string[] = [];
    for (const line of raw.split("\n")) {
      if (line.startsWith("data:")) {
        dataLines.push(line.slice(5).replace(/^ /, ""));
      }
    }
    if (dataLines.length === 0) return;
    const payload = dataLines.join("\n");
    if (payload === "" || payload === "[DONE]") return;
    try {
      const ev = JSON.parse(payload) as StreamEvent;
      onEvent(ev);
    } catch {
      // ignore malformed event
    }
  };

  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, { stream: true });
    let sep: number;
    // SSE events are delimited by a blank line (\n\n).
    while ((sep = buffer.indexOf("\n\n")) !== -1) {
      const chunk = buffer.slice(0, sep);
      buffer = buffer.slice(sep + 2);
      flush(chunk);
    }
  }
  if (buffer.trim() !== "") flush(buffer);
}
