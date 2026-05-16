import { useEffect, useMemo, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { api, ApiError, type ModelsInfo } from "../api";
import { ErrorBanner, Spinner } from "../components/common";
import { useData } from "../data";

// Start a conversation scoped to one Connection. The Connection is fixed by
// the route (/connections/:connId/new-conversation); only the model is
// chosen here.
export default function NewChatPage() {
  const { connId } = useParams();
  const navigate = useNavigate();
  const { connections, reload, connectionById } = useData();

  const [models, setModels] = useState<ModelsInfo | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [model, setModel] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  useEffect(() => {
    api
      .models()
      .then(setModels)
      .catch((e) =>
        setLoadError(
          e instanceof ApiError ? e.message : "Failed to load models",
        ),
      );
  }, []);

  // Flatten "<provider>:<model>" choices across all providers with a key.
  const modelChoices = useMemo(() => {
    if (!models) return [];
    const out: { value: string; label: string }[] = [];
    for (const p of models.system_providers) {
      if (!p.has_key) continue;
      for (const m of p.models) {
        out.push({ value: m, label: `${m} (${p.id})` });
      }
    }
    for (const k of models.byo_keys) {
      out.push({
        value: `byo:${k.id}`,
        label: `${k.label} — your ${k.provider_kind} key`,
      });
    }
    return out;
  }, [models]);

  if (!connections || (!models && !loadError)) return <Spinner />;

  const conn = connId ? connectionById(connId) : undefined;
  if (!conn) {
    return (
      <div className="page">
        <div className="page-head">
          <div>
            <h1>Connection not found</h1>
            <p>Pick a connection to start a conversation.</p>
          </div>
        </div>
        <Link to="/">Back to home</Link>
      </div>
    );
  }

  const onCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitting(true);
    setFormError(null);
    try {
      // `source_id` is the wire field — unchanged by the UI relabel.
      const body: { source_id: string; model?: string } = {
        source_id: conn.id,
      };
      if (model) body.model = model;
      const { chat } = await api.createChat(body);
      await reload();
      navigate(`/chats/${chat.id}`);
    } catch (e) {
      setFormError(
        e instanceof ApiError ? e.message : "Failed to create chat",
      );
      setSubmitting(false);
    }
  };

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>New conversation</h1>
          <p>
            With connection <strong>{conn.label}</strong>. Pick a model to
            begin.
          </p>
        </div>
      </div>

      {loadError && <ErrorBanner message={loadError} />}

      <form className="card newchat-form" onSubmit={onCreate}>
        <div style={{ padding: 24 }}>
          <div className="field">
            <label htmlFor="model">Model</label>
            <select
              id="model"
              value={model}
              onChange={(e) => setModel(e.target.value)}
            >
              <option value="">
                Deployment default
                {models?.default_provider
                  ? ` (${models.default_provider})`
                  : ""}
              </option>
              {modelChoices.map((c) => (
                <option key={c.value} value={c.value}>
                  {c.label}
                </option>
              ))}
            </select>
          </div>

          {formError && <ErrorBanner message={formError} />}

          <div className="row" style={{ marginTop: 16 }}>
            <button type="submit" className="primary" disabled={submitting}>
              {submitting ? "Starting…" : "Start conversation"}
            </button>
            <button
              type="button"
              className="ghost"
              onClick={() => navigate(`/connections/${conn.id}`)}
            >
              Cancel
            </button>
          </div>
        </div>
      </form>
    </div>
  );
}
