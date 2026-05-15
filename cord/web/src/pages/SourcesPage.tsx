import { useEffect, useState } from "react";
import { api, ApiError, type Source } from "../api";
import {
  Empty,
  ErrorBanner,
  formatDate,
  Spinner,
  StatusPill,
} from "../components/common";

type ConnectMode = "link" | "direct";

export default function SourcesPage() {
  const [sources, setSources] = useState<Source[] | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);

  const load = () => {
    api
      .sources()
      .then((d) => {
        setSources(d.sources);
        setLoadError(null);
      })
      .catch((e) =>
        setLoadError(
          e instanceof ApiError ? e.message : "Failed to load sources",
        ),
      );
  };

  useEffect(load, []);

  const onRefresh = async (id: string) => {
    setBusyId(id);
    try {
      const { source } = await api.refreshSource(id);
      setSources((ss) =>
        ss ? ss.map((s) => (s.id === id ? source : s)) : ss,
      );
    } catch (e) {
      alert(e instanceof ApiError ? e.message : "Refresh failed");
    } finally {
      setBusyId(null);
    }
  };

  const onDisconnect = async (id: string) => {
    if (!confirm("Disconnect this source? The stored credential is wiped."))
      return;
    setBusyId(id);
    try {
      await api.deleteSource(id);
      setSources((ss) => (ss ? ss.filter((s) => s.id !== id) : ss));
    } catch (e) {
      alert(e instanceof ApiError ? e.message : "Disconnect failed");
    } finally {
      setBusyId(null);
    }
  };

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Data sources</h1>
          <p>Connected health-data stores CORD may read on your behalf.</p>
        </div>
      </div>

      <ConnectForm onConnected={load} />

      {loadError && <ErrorBanner message={loadError} />}
      {!sources && !loadError && <Spinner />}

      {sources && sources.length === 0 && (
        <Empty>No data sources connected yet.</Empty>
      )}

      {sources && sources.length > 0 && (
        <div className="stack" style={{ marginTop: 18 }}>
          {sources.map((s) => (
            <div key={s.id} className="list-item">
              <div className="spread">
                <div>
                  <strong>{s.label}</strong>{" "}
                  <StatusPill status={s.status} />
                </div>
                <div className="row">
                  <button
                    className="small"
                    disabled={busyId === s.id}
                    onClick={() => onRefresh(s.id)}
                  >
                    Refresh
                  </button>
                  <button
                    className="small danger"
                    disabled={busyId === s.id}
                    onClick={() => onDisconnect(s.id)}
                  >
                    Disconnect
                  </button>
                </div>
              </div>
              <div
                className="muted"
                style={{ marginTop: 6, fontSize: 12.5 }}
              >
                <div>
                  {s.kind} · {s.endpoint}
                </div>
                <div>
                  Connected {formatDate(s.created_at)} · Last reachable{" "}
                  {formatDate(s.last_ok_at)}
                </div>
                {s.scope_json && (
                  <div className="faint" style={{ marginTop: 4 }}>
                    Scope: {s.scope_json}
                  </div>
                )}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function ConnectForm({ onConnected }: { onConnected: () => void }) {
  const [mode, setMode] = useState<ConnectMode>("link");
  const [label, setLabel] = useState("");
  const [link, setLink] = useState("");
  const [endpoint, setEndpoint] = useState("");
  const [token, setToken] = useState("");
  const [pin, setPin] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const reset = () => {
    setLabel("");
    setLink("");
    setEndpoint("");
    setToken("");
    setPin("");
  };

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      if (mode === "link") {
        await api.connectSource({
          link: link.trim(),
          ...(label.trim() ? { label: label.trim() } : {}),
        });
      } else {
        await api.connectSource({
          endpoint: endpoint.trim(),
          token: token.trim(),
          ...(pin.trim() ? { pin: pin.trim() } : {}),
          ...(label.trim() ? { label: label.trim() } : {}),
        });
      }
      reset();
      onConnected();
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Connection failed");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <form className="card" onSubmit={submit} style={{ marginBottom: 8 }}>
      <div style={{ padding: 20 }}>
        <h2 style={{ marginBottom: 12 }}>Connect a source</h2>

        <div className="row" style={{ marginBottom: 14 }}>
          <button
            type="button"
            className={mode === "link" ? "primary small" : "small"}
            onClick={() => setMode("link")}
          >
            Share link
          </button>
          <button
            type="button"
            className={mode === "direct" ? "primary small" : "small"}
            onClick={() => setMode("direct")}
          >
            Direct connection
          </button>
        </div>

        <div className="field">
          <label htmlFor="label">Label (optional)</label>
          <input
            id="label"
            value={label}
            onChange={(e) => setLabel(e.target.value)}
            placeholder="e.g. My phone"
          />
        </div>

        {mode === "link" ? (
          <div className="field">
            <label htmlFor="link">Share link</label>
            <input
              id="link"
              value={link}
              onChange={(e) => setLink(e.target.value)}
              placeholder="ohd://share/…"
              required
            />
          </div>
        ) : (
          <>
            <div className="field">
              <label htmlFor="endpoint">Endpoint</label>
              <input
                id="endpoint"
                value={endpoint}
                onChange={(e) => setEndpoint(e.target.value)}
                placeholder="https://storage.example/…"
                required
              />
            </div>
            <div className="field">
              <label htmlFor="token">Grant token</label>
              <input
                id="token"
                value={token}
                onChange={(e) => setToken(e.target.value)}
                placeholder="ohdg_…"
                required
              />
            </div>
            <div className="field">
              <label htmlFor="pin">Certificate pin (optional)</label>
              <input
                id="pin"
                value={pin}
                onChange={(e) => setPin(e.target.value)}
                placeholder="SPKI hash"
              />
            </div>
          </>
        )}

        {error && <ErrorBanner message={error} />}

        <div style={{ marginTop: 12 }}>
          <button type="submit" className="primary" disabled={submitting}>
            {submitting ? "Connecting…" : "Connect"}
          </button>
        </div>
      </div>
    </form>
  );
}
