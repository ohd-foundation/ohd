import { useEffect, useState } from "react";
import { api, ApiError, type ModelsInfo } from "../api";
import {
  Empty,
  ErrorBanner,
  formatDate,
  Spinner,
} from "../components/common";

type ProviderKind = "anthropic" | "gemini" | "openai";

export default function ModelsPage() {
  const [info, setInfo] = useState<ModelsInfo | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const load = () => {
    api
      .models()
      .then((d) => {
        setInfo(d);
        setLoadError(null);
      })
      .catch((e) =>
        setLoadError(
          e instanceof ApiError ? e.message : "Failed to load models",
        ),
      );
  };

  useEffect(load, []);

  if (loadError) {
    return (
      <div className="page">
        <ErrorBanner message={loadError} />
      </div>
    );
  }
  if (!info) return <Spinner />;

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Models</h1>
          <p>Model providers available for conversations on this deployment.</p>
        </div>
      </div>

      <h2 style={{ marginBottom: 10 }}>System providers</h2>
      {info.system_providers.length === 0 ? (
        <Empty>No system providers configured.</Empty>
      ) : (
        <div className="stack">
          {info.system_providers.map((p) => (
            <div key={p.id} className="list-item">
              <div className="spread">
                <strong>{p.id}</strong>
                <span className={p.has_key ? "pill ok" : "pill warn"}>
                  {p.has_key ? "key configured" : "no key"}
                </span>
              </div>
              <div className="muted" style={{ marginTop: 6, fontSize: 12.5 }}>
                {p.kind} ·{" "}
                {p.models.length > 0 ? p.models.join(", ") : "no models"}
                {info.default_provider === p.id && " · deployment default"}
              </div>
            </div>
          ))}
        </div>
      )}

      <h2 style={{ margin: "26px 0 10px" }}>Your provider keys</h2>
      {info.allow_user_keys ? (
        <>
          <ByoKeyList info={info} onChange={load} />
          <AddKeyForm onAdded={load} />
        </>
      ) : (
        <div className="banner info">
          This deployment does not allow bring-your-own model keys. All
          inference runs on the operator's configured provider.
        </div>
      )}
    </div>
  );
}

function ByoKeyList({
  info,
  onChange,
}: {
  info: ModelsInfo;
  onChange: () => void;
}) {
  const [busyId, setBusyId] = useState<string | null>(null);

  if (info.byo_keys.length === 0) {
    return <Empty>You have not added any provider keys.</Empty>;
  }

  const remove = async (id: string) => {
    if (!confirm("Delete this key?")) return;
    setBusyId(id);
    try {
      await api.deleteByoKey(id);
      onChange();
    } catch (e) {
      alert(e instanceof ApiError ? e.message : "Failed to delete key");
    } finally {
      setBusyId(null);
    }
  };

  return (
    <div className="stack" style={{ marginBottom: 16 }}>
      {info.byo_keys.map((k) => (
        <div key={k.id} className="list-item">
          <div className="spread">
            <div>
              <strong>{k.label}</strong>{" "}
              <span className="pill">{k.provider_kind}</span>
            </div>
            <button
              className="small danger"
              disabled={busyId === k.id}
              onClick={() => remove(k.id)}
            >
              Delete
            </button>
          </div>
          <div className="faint" style={{ marginTop: 6, fontSize: 12.5 }}>
            Added {formatDate(k.created_at)}
          </div>
        </div>
      ))}
    </div>
  );
}

function AddKeyForm({ onAdded }: { onAdded: () => void }) {
  const [providerKind, setProviderKind] = useState<ProviderKind>("anthropic");
  const [label, setLabel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      await api.addByoKey({
        provider_kind: providerKind,
        label: label.trim(),
        api_key: apiKey.trim(),
      });
      setLabel("");
      setApiKey("");
      onAdded();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Failed to add key");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <form className="card" onSubmit={submit}>
      <div style={{ padding: 20 }}>
        <h3 style={{ marginBottom: 12 }}>Add a provider key</h3>

        <div className="field">
          <label htmlFor="kind">Provider</label>
          <select
            id="kind"
            value={providerKind}
            onChange={(e) => setProviderKind(e.target.value as ProviderKind)}
          >
            <option value="anthropic">Anthropic</option>
            <option value="gemini">Gemini</option>
            <option value="openai">OpenAI</option>
          </select>
        </div>

        <div className="field">
          <label htmlFor="keylabel">Label</label>
          <input
            id="keylabel"
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            placeholder="e.g. Personal Anthropic key"
            required
          />
        </div>

        <div className="field">
          <label htmlFor="apikey">API key</label>
          <input
            id="apikey"
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            placeholder="Stored encrypted at rest"
            required
          />
        </div>

        {error && <ErrorBanner message={error} />}

        <div style={{ marginTop: 12 }}>
          <button type="submit" className="primary" disabled={submitting}>
            {submitting ? "Adding…" : "Add key"}
          </button>
        </div>
      </div>
    </form>
  );
}
