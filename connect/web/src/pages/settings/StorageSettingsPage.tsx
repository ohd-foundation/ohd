import { useState } from "react";
import { useToast } from "../../components/Toast";
import {
  resolveSelfToken,
  resolveStorageUrl,
  setSelfToken,
  setStorageUrl,
} from "../../ohdc/client";
import { getMyUserUlid, getSnapshot, reBootstrap } from "../../ohdc/store";
import { useStoreVersion } from "../../ohdc/useStore";

/**
 * Settings → Storage. Lets the user:
 *   - See the current storage URL + protocol / version readout from `Health`.
 *   - Change the storage URL (sessionStorage; clears on tab close).
 *   - Paste a new self-session token (sessionStorage).
 *   - "Switch deployment" — informational; the OAuth bootstrap is v0.x.
 */
export function StorageSettingsPage() {
  useStoreVersion();
  const snap = getSnapshot();
  const toast = useToast();

  const [url, setUrl] = useState(resolveStorageUrl());
  const [token, setToken] = useState("");
  const userUlid = getMyUserUlid();
  const tokenPresent = !!resolveSelfToken();

  const onSaveUrl = () => {
    setStorageUrl(url.trim());
    toast.show("Storage URL updated.", "success");
    void reBootstrap();
  };
  const onSaveToken = () => {
    if (!token.trim()) return;
    setSelfToken(token.trim());
    toast.show("Self-session token saved. Bootstrapping…", "success");
    setToken("");
    void reBootstrap();
  };

  return (
    <div data-testid="settings-storage">
      <div className="card">
        <div className="card-title">
          <h3>Storage URL</h3>
        </div>
        <p className="muted" style={{ marginTop: 0 }}>
          The OHD Storage instance this app talks to. Default is{" "}
          <code>http://localhost:8443</code> for a locally-running{" "}
          <code>ohd-storage-server serve</code>.
        </p>
        <label className="field">
          URL
          <input type="url" className="mono" value={url} onChange={(e) => setUrl(e.target.value)} />
        </label>
        <div style={{ marginTop: 12 }}>
          <button className="btn btn-primary" onClick={onSaveUrl}>
            Save URL
          </button>
        </div>
      </div>

      <div className="card">
        <div className="card-title">
          <h3>Self-session token</h3>
          <span className={`flag ${tokenPresent ? "flag-success" : "flag-warn"}`}>
            {tokenPresent ? "set" : "missing"}
          </span>
        </div>
        <p className="muted" style={{ marginTop: 0 }}>
          The <code>ohds_…</code> token issued by your storage instance. The proper OAuth
          flow lands in v0.x; for now, paste a token here. Stored in <code>sessionStorage</code>
          and cleared when you close the tab.
        </p>
        <label className="field">
          Token
          <input
            type="text"
            className="mono"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder="ohds_…"
          />
        </label>
        <div style={{ marginTop: 12 }}>
          <button className="btn btn-primary" onClick={onSaveToken} disabled={!token.trim()}>
            Save token
          </button>
        </div>
      </div>

      <div className="card">
        <div className="card-title">
          <h3>Server status</h3>
        </div>
        <dl className="kv-grid">
          <dt>Status</dt>
          <dd>{snap.health?.status ?? "unknown"}</dd>
          <dt>Server version</dt>
          <dd className="mono">{snap.health?.serverVersion ?? "?"}</dd>
          <dt>Protocol</dt>
          <dd className="mono">{snap.health?.protocolVersion ?? "?"}</dd>
          <dt>Server time</dt>
          <dd className="mono">{snap.health?.serverTimeMs ? new Date(snap.health.serverTimeMs).toISOString() : "?"}</dd>
          <dt>Your user ULID</dt>
          <dd className="mono">{userUlid || "?"}</dd>
          <dt>Token kind</dt>
          <dd className="mono">{snap.me?.tokenKind ?? "?"}</dd>
        </dl>
      </div>

      <div className="card">
        <div className="card-title">
          <h3>Switch deployment</h3>
        </div>
        <p className="muted" style={{ marginTop: 0 }}>
          Migrating between deployment modes (on-device → cloud, cloud → self-hosted, etc.) uses
          the storage <code>Export.MigrateInit</code> + <code>MigrateFinalize</code> RPCs.
          These are stubbed in the current storage build; this affordance will activate once
          they ship. See <a href="/settings/export">Export</a> for the manual path today.
        </p>
        <button className="btn" disabled>
          Switch deployment (coming soon)
        </button>
      </div>
    </div>
  );
}
