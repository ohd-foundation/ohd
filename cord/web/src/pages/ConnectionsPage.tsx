import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { api, ApiError } from "../api";
import { ErrorBanner } from "../components/common";
import { useData } from "../data";

type ConnectMode = "link" | "direct";

// Add a connection: links CORD to a health-data store, either from a share
// link or by direct endpoint + grant token.
export default function ConnectionsPage() {
  const navigate = useNavigate();
  const { reload } = useData();

  const [mode, setMode] = useState<ConnectMode>("link");
  const [label, setLabel] = useState("");
  const [link, setLink] = useState("");
  const [endpoint, setEndpoint] = useState("");
  const [token, setToken] = useState("");
  const [pin, setPin] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setSubmitting(true);
    setError(null);
    try {
      let created;
      if (mode === "link") {
        created = await api.addConnection({
          link: link.trim(),
          ...(label.trim() ? { label: label.trim() } : {}),
        });
      } else {
        created = await api.addConnection({
          endpoint: endpoint.trim(),
          token: token.trim(),
          ...(pin.trim() ? { pin: pin.trim() } : {}),
          ...(label.trim() ? { label: label.trim() } : {}),
        });
      }
      await reload();
      navigate(`/connections/${created.source.id}`);
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Connection failed");
      setSubmitting(false);
    }
  };

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>Add a connection</h1>
          <p>Link CORD to a health-data store it may read on your behalf.</p>
        </div>
      </div>

      <form className="card newchat-form" onSubmit={submit}>
        <div style={{ padding: 24 }}>
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

          <div className="row" style={{ marginTop: 16 }}>
            <button type="submit" className="primary" disabled={submitting}>
              {submitting ? "Connecting…" : "Add connection"}
            </button>
            <button
              type="button"
              className="ghost"
              onClick={() => navigate("/")}
            >
              Cancel
            </button>
          </div>
        </div>
      </form>
    </div>
  );
}
