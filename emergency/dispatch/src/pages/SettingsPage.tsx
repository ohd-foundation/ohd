import { useState } from "react";
import {
  forgetOperatorToken,
  resolveOperatorToken,
  resolveStorageUrl,
  setOperatorToken,
  setStorageUrl,
} from "../ohdc/client";
import { getSession, isMockMode } from "../mock/store";
import { fmtStamp } from "../util";

export function SettingsPage() {
  const session = getSession();
  const [storageUrl, setStorageUrlState] = useState(resolveStorageUrl());
  const [token, setTokenState] = useState(resolveOperatorToken() ?? "");
  const [relayUrl, setRelayUrl] = useState(
    () => localStorage.getItem("ohd-dispatch-relay-url") ?? "",
  );
  const [stationLabel, setStationLabel] = useState(
    () => localStorage.getItem("ohd-dispatch-station-label") ?? session.station_label,
  );
  const [pushProvider, setPushProvider] = useState(
    () => localStorage.getItem("ohd-dispatch-push-provider") ?? "fcm",
  );
  const [saved, setSaved] = useState<string | null>(null);

  function save() {
    setStorageUrl(storageUrl.trim());
    if (token.trim()) {
      setOperatorToken(token.trim());
    } else {
      forgetOperatorToken();
    }
    localStorage.setItem("ohd-dispatch-relay-url", relayUrl.trim());
    localStorage.setItem("ohd-dispatch-station-label", stationLabel.trim());
    localStorage.setItem("ohd-dispatch-push-provider", pushProvider);
    setSaved("Settings saved. Reloading…");
    setTimeout(() => window.location.reload(), 600);
  }

  return (
    <div className="page" data-testid="settings-page">
      <header className="page-head">
        <div>
          <h1>Settings</h1>
          <p className="muted">Local browser settings + operator session info.</p>
        </div>
      </header>

      <section className="panel">
        <header className="panel-head">
          <h2>Storage + relay</h2>
        </header>
        <div className="form-grid">
          <label className="field">
            <span>Storage URL</span>
            <input
              className="input"
              value={storageUrl}
              onChange={(e) => setStorageUrlState(e.target.value)}
              placeholder="https://storage.ems-prague.cz"
            />
            <small className="muted">OHDC Connect-RPC base URL.</small>
          </label>
          <label className="field">
            <span>Operator token</span>
            <textarea
              className="input"
              value={token}
              onChange={(e) => setTokenState(e.target.value)}
              placeholder="ohdg_… or operator session bearer"
              rows={3}
            />
            <small className="muted">
              Stored in <code>localStorage</code>. Issued by the operator IdP /
              relay; carries the dispatcher's authority to read cases bound to
              this station.
            </small>
          </label>
          <label className="field">
            <span>Relay URL</span>
            <input
              className="input"
              value={relayUrl}
              onChange={(e) => setRelayUrl(e.target.value)}
              placeholder="https://relay.ems-prague.cz"
            />
          </label>
          <label className="field">
            <span>Station label</span>
            <input
              className="input"
              value={stationLabel}
              onChange={(e) => setStationLabel(e.target.value)}
              placeholder="EMS Prague Region — Central"
            />
            <small className="muted">
              Mirrored from the authority cert CN; shown in the top bar.
            </small>
          </label>
        </div>
      </section>

      <section className="panel">
        <header className="panel-head">
          <h2>Authority cert</h2>
        </header>
        <dl className="kv">
          <div>
            <dt>Subject</dt>
            <dd className="mono">{session.authority_cert_subject}</dd>
          </div>
          <div>
            <dt>Fingerprint</dt>
            <dd className="mono">{session.authority_cert_fingerprint}</dd>
          </div>
          <div>
            <dt>Expires</dt>
            <dd className="mono">{fmtStamp(session.authority_cert_expires_at_ms)}</dd>
          </div>
        </dl>
        <p className="muted footnote">
          Authority cert is issued daily by the relay's Fulcio integration.
          The dispatch console reads it from the relay's <code>/healthz/cert</code>
          endpoint (TBD in v0).
        </p>
      </section>

      <section className="panel">
        <header className="panel-head">
          <h2>Push notifications</h2>
        </header>
        <label className="field field-row">
          <span>Provider</span>
          <select
            className="input"
            value={pushProvider}
            onChange={(e) => setPushProvider(e.target.value)}
          >
            <option value="fcm">Firebase Cloud Messaging (Android)</option>
            <option value="apns">Apple Push (APNs)</option>
            <option value="webpush">Web Push (VAPID)</option>
          </select>
        </label>
        <p className="muted footnote">
          Provider config; secret credentials live in the relay env (not
          here). v0 stub.
        </p>
      </section>

      <div className="panel-actions">
        <button type="button" className="btn btn-primary" onClick={save}>
          Save and reload
        </button>
        <button
          type="button"
          className="btn btn-ghost"
          onClick={() => {
            forgetOperatorToken();
            window.location.reload();
          }}
        >
          Forget token
        </button>
        {saved && <span className="saved-msg">{saved}</span>}
      </div>

      {isMockMode && (
        <p className="footnote muted">
          MOCK MODE active (<code>VITE_USE_MOCK=1</code>): saved values are
          stored locally but the OHDC client is not used.
        </p>
      )}
    </div>
  );
}
