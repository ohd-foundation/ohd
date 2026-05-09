// SettingsMcpPage — operator settings for the Care MCP + LLM endpoint.
//
// Per `care/SPEC.md` §10 the operator picks where to point Care's chat
// panel. Defaults come from `VITE_*` env vars at build time; runtime
// overrides land here and persist in `localStorage` via `mcp/settings.ts`.
//
// Fields:
//   - MCP URL — `https://<your-mcp-host>/mcp` per FastMCP's
//     Streamable HTTP convention.
//   - LLM URL — OpenAI-compatible base URL (without `/chat/completions`).
//   - LLM API key — Bearer for the LLM endpoint. Stored in
//     localStorage; "Sign out" wipes it.
//   - LLM model — id, e.g. "gpt-4.1-mini".
//   - "PHI not sent to external LLMs" toggle — informational; the
//     real enforcement lives in the Care MCP server's
//     `OHD_CARE_NO_PHI_TO_EXTERNAL_LLMS=1` env var. Surfacing it here
//     lets the operator confirm the deployment posture before they
//     start a chat that quotes patient data.
//
// We deliberately do NOT mirror the MCP server's OIDC client_id /
// upstream issuer here — those are deployment-time config. The
// operator authenticates via the same OIDC flow Care/web uses for
// OHDC; that bearer is what we forward to the MCP server.

import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { useToast } from "../components/Toast";
import {
  clearMcpSettings,
  loadMcpSettings,
  saveMcpSettings,
  type McpSettings,
} from "../mcp/settings";

export function SettingsMcpPage() {
  const toast = useToast();
  const [settings, setSettings] = useState<McpSettings>(loadMcpSettings);

  useEffect(() => {
    setSettings(loadMcpSettings());
  }, []);

  const onSave = useCallback(() => {
    saveMcpSettings(settings);
    toast.show("Settings saved", "success");
  }, [settings, toast]);

  const onClear = useCallback(() => {
    clearMcpSettings();
    setSettings(loadMcpSettings());
    toast.show("Settings cleared");
  }, [toast]);

  return (
    <div className="settings-page" data-testid="settings-mcp-page">
      <header>
        <h2>Settings → MCP</h2>
        <p className="muted">
          Configure where care/web's chat panel routes tool calls and LLM
          completions. Per <code>care/SPEC.md</code> §10. The operator OIDC
          bearer obtained at sign-in is forwarded to the MCP endpoint
          automatically.
        </p>
      </header>

      <label className="form-row">
        <span>MCP endpoint URL</span>
        <input
          type="url"
          value={settings.mcpUrl}
          onChange={(e) =>
            setSettings((s) => ({ ...s, mcpUrl: e.target.value }))
          }
          placeholder="https://care.clinic.example.com/mcp"
          data-testid="settings-mcp-url"
        />
        <span className="muted" style={{ fontSize: 11 }}>
          The Care MCP server's Streamable HTTP endpoint. Defaults to{" "}
          <code>VITE_MCP_URL</code> at build time.
        </span>
      </label>

      <label className="form-row">
        <span>LLM endpoint URL</span>
        <input
          type="url"
          value={settings.llmUrl}
          onChange={(e) =>
            setSettings((s) => ({ ...s, llmUrl: e.target.value }))
          }
          placeholder="https://api.openai.com/v1"
          data-testid="settings-llm-url"
        />
        <span className="muted" style={{ fontSize: 11 }}>
          OpenAI-compatible base URL — <code>/chat/completions</code> is
          appended. Local backends (Ollama, vLLM, llama.cpp) work too.
        </span>
      </label>

      <label className="form-row">
        <span>LLM API key</span>
        <input
          type="password"
          value={settings.llmApiKey}
          onChange={(e) =>
            setSettings((s) => ({ ...s, llmApiKey: e.target.value }))
          }
          placeholder="sk-..."
          data-testid="settings-llm-key"
          autoComplete="off"
        />
        <span className="muted" style={{ fontSize: 11 }}>
          Stored in this browser's localStorage. "Sign out" wipes it.
        </span>
      </label>

      <label className="form-row">
        <span>LLM model</span>
        <input
          type="text"
          value={settings.llmModel}
          onChange={(e) =>
            setSettings((s) => ({ ...s, llmModel: e.target.value }))
          }
          placeholder="gpt-4.1-mini"
          data-testid="settings-llm-model"
        />
      </label>

      <label
        className="form-row"
        style={{ flexDirection: "row", alignItems: "center", gap: 8 }}
      >
        <input
          type="checkbox"
          checked={settings.noPhiToExternalLlms}
          onChange={(e) =>
            setSettings((s) => ({ ...s, noPhiToExternalLlms: e.target.checked }))
          }
          data-testid="settings-no-phi"
        />
        <span>
          PHI does not flow to this LLM (deployment sets{" "}
          <code>OHD_CARE_NO_PHI_TO_EXTERNAL_LLMS=1</code> on the MCP)
        </span>
      </label>

      <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
        <button
          type="button"
          className="btn btn-primary"
          onClick={onSave}
          data-testid="settings-save"
        >
          Save
        </button>
        <button
          type="button"
          className="btn btn-ghost"
          onClick={onClear}
          data-testid="settings-clear"
        >
          Clear
        </button>
        <Link to="/chat" className="btn btn-ghost" style={{ marginLeft: "auto" }}>
          Back to chat
        </Link>
      </div>
    </div>
  );
}
