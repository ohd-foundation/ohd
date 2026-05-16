import { useState } from "react";
import {
  beginLogin,
  defaultOidcConfig,
  OHD_ACCOUNT_PROVIDER,
  type OidcConfig,
} from "../ohdc/oidc";

/**
 * Self-session sign-in page. Starts the OAuth 2.0 Authorization Code +
 * PKCE flow against the user's OHD Storage instance. Storage takes
 * over the upstream OIDC step (Google / Apple / OHD Account / etc.) per
 * `spec/docs/design/auth.md` "Role split"; the SPA never sees the
 * upstream id_token.
 *
 * Two entry points:
 *  - **Sign in with OHD** deep-links the storage AS straight to OHD's
 *    first-party identity provider (`accounts.ohd.dev`) — the no-big-tech
 *    path. This is the primary action for OHD Cloud users.
 *  - **Other sign-in options** lands on the storage AS's own login page,
 *    which lists whatever providers the operator enabled.
 */
export function LoginPage() {
  const [config, setConfig] = useState<OidcConfig>(() => defaultOidcConfig());
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<null | "ohd" | "other">(null);
  const isConfigured = !!config.storageUrl && !!config.clientId;

  const handleSignIn = async (providerHint?: string) => {
    setError(null);
    setPending(providerHint ? "ohd" : "other");
    try {
      await beginLogin(config, providerHint);
    } catch (err) {
      setPending(null);
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="empty">
      <h2>Sign in to OHD Connect</h2>
      <p className="muted">
        OHD Connect signs you in via your OHD Storage instance, which
        delegates to whichever identity provider you (or your operator)
        configured — OHD Account, Google, Apple, Microsoft, GitHub, or
        a custom OIDC issuer. We use OAuth 2.0 Authorization Code +
        PKCE; no passwords are sent to this app.
      </p>

      <div className="form" style={{ maxWidth: 480 }}>
        <label>
          Storage URL
          <input
            value={config.storageUrl}
            onChange={(e) => setConfig({ ...config, storageUrl: e.target.value })}
            placeholder="https://ohd.cloud.example"
          />
        </label>
        <label>
          OAuth client_id
          <input
            value={config.clientId}
            onChange={(e) => setConfig({ ...config, clientId: e.target.value })}
            placeholder="ohd-connect-web"
          />
        </label>
        <label>
          Redirect URI
          <input
            value={config.redirectUri}
            onChange={(e) => setConfig({ ...config, redirectUri: e.target.value })}
          />
        </label>
        <label>
          Scopes
          <input
            value={config.scope ?? ""}
            onChange={(e) => setConfig({ ...config, scope: e.target.value })}
          />
        </label>
        <button
          type="button"
          className="btn btn-primary"
          disabled={!isConfigured || pending !== null}
          onClick={() => handleSignIn(OHD_ACCOUNT_PROVIDER)}
        >
          {pending === "ohd" ? "Redirecting…" : "Sign in with OHD"}
        </button>
        <button
          type="button"
          className="btn"
          disabled={!isConfigured || pending !== null}
          onClick={() => handleSignIn()}
        >
          {pending === "other" ? "Redirecting…" : "Other sign-in options"}
        </button>
        {error && (
          <p className="error" style={{ color: "var(--danger, #b00020)" }}>
            {error}
          </p>
        )}
        <p className="muted" style={{ fontSize: 12 }}>
          <b>Sign in with OHD</b> uses OHD's own identity provider
          (<code>accounts.ohd.dev</code>) — no big-tech account needed.{" "}
          <b>Other sign-in options</b> shows your storage operator's full
          provider list. Set <code>VITE_OIDC_STORAGE_URL</code>,{" "}
          <code>VITE_OIDC_CLIENT_ID</code>,{" "}
          <code>VITE_OIDC_REDIRECT_URI</code>, and{" "}
          <code>VITE_OIDC_SCOPE</code> at build time to skip this form.
        </p>
      </div>
    </div>
  );
}
