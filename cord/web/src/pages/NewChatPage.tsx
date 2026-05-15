import { useEffect, useMemo, useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import {
  api,
  ApiError,
  type ModelsInfo,
  type Source,
} from "../api";
import { ErrorBanner, Spinner } from "../components/common";

// Landing pane: pick a source + model and start a conversation.
export default function NewChatPage() {
  const navigate = useNavigate();
  const [sources, setSources] = useState<Source[] | null>(null);
  const [models, setModels] = useState<ModelsInfo | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [sourceId, setSourceId] = useState("");
  const [model, setModel] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  useEffect(() => {
    Promise.all([api.sources(), api.models()])
      .then(([s, m]) => {
        setSources(s.sources);
        setModels(m);
        if (s.sources.length > 0) setSourceId(s.sources[0].id);
      })
      .catch((e) =>
        setLoadError(
          e instanceof ApiError ? e.message : "Failed to load setup data",
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

  const onCreate = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!sourceId) return;
    setSubmitting(true);
    setFormError(null);
    try {
      const body: { source_id: string; model?: string } = {
        source_id: sourceId,
      };
      if (model) body.model = model;
      const { chat } = await api.createChat(body);
      navigate(`/chats/${chat.id}`);
    } catch (e) {
      setFormError(
        e instanceof ApiError ? e.message : "Failed to create chat",
      );
      setSubmitting(false);
    }
  };

  if (!sources && !loadError) return <Spinner />;

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>New conversation</h1>
          <p>Pick a data source and a model to begin.</p>
        </div>
      </div>

      {loadError && <ErrorBanner message={loadError} />}

      {sources && sources.length === 0 && (
        <div className="banner info">
          You have no connected data sources yet.{" "}
          <Link to="/sources">Connect a source</Link> to start a conversation.
        </div>
      )}

      {sources && sources.length > 0 && (
        <form className="card newchat-form" onSubmit={onCreate}>
          <div style={{ padding: 24 }}>
            <div className="field">
              <label htmlFor="src">Data source</label>
              <select
                id="src"
                value={sourceId}
                onChange={(e) => setSourceId(e.target.value)}
              >
                {sources.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.label} — {s.status}
                  </option>
                ))}
              </select>
            </div>

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

            <div style={{ marginTop: 16 }}>
              <button
                type="submit"
                className="primary"
                disabled={submitting || !sourceId}
              >
                {submitting ? "Starting…" : "Start conversation"}
              </button>
            </div>
          </div>
        </form>
      )}
    </div>
  );
}
