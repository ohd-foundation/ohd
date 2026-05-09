import { useState } from "react";
import { beginLogin, defaultOidcConfig, type OidcConfig } from "../ohdc/oidc";

/**
 * Operator-OIDC sign-in page for the dispatch console.
 *
 * Mirrors `connect/web/src/pages/LoginPage.tsx`. Differences:
 *   - The IdP here is the **operator's** identity provider (Keycloak /
 *     Authentik / Auth0 / Azure AD), not the patient's storage AS.
 *   - The token is persisted to localStorage (operator hardware) under
 *     the existing `ohd-dispatch-operator-token` key, so the rest of
 *     the SPA's OHDC client picks it up unchanged.
 *
 * Per `spec/docs/design/auth.md` "Operator authentication", the
 * dispatcher signs in against their station's IdP via OAuth 2.0
 * Authorization Code + PKCE; the issued bearer carries the operator's
 * authority (cases under this station's jurisdiction).
 */
export function LoginPage() {
  const [config, setConfig] = useState<OidcConfig>(() => defaultOidcConfig());
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const isConfigured = !!config.issuer && !!config.clientId;

  const handleSignIn = async () => {
    setError(null);
    setPending(true);
    try {
      await beginLogin(config);
    } catch (err) {
      setPending(false);
      setError(err instanceof Error ? err.message : String(err));
    }
  };

  return (
    <div className="page" data-testid="login-page">
      <header className="page-head">
        <div>
          <h1>Sign in to dispatch</h1>
          <p className="muted">
            OHD Emergency dispatch authenticates against your operator's
            identity provider (Keycloak / Authentik / Auth0 / Azure AD)
            via OAuth 2.0 Authorization Code + PKCE. No passwords are
            sent to this app.
          </p>
        </div>
      </header>

      <section className="panel">
        <header className="panel-head">
          <h2>OIDC issuer</h2>
        </header>
        <div className="form-grid">
          <label className="field">
            <span>Issuer URL</span>
            <input
              className="input"
              value={config.issuer}
              onChange={(e) => setConfig({ ...config, issuer: e.target.value })}
              placeholder="https://idp.ems-prague.cz/realms/dispatch"
            />
            <small className="muted">
              The operator IdP's discovery root; we hit
              <code> /.well-known/oauth-authorization-server</code> (or
              <code> /.well-known/openid-configuration</code>) under it.
            </small>
          </label>
          <label className="field">
            <span>Client ID</span>
            <input
              className="input"
              value={config.clientId}
              onChange={(e) => setConfig({ ...config, clientId: e.target.value })}
              placeholder="ohd-emergency-dispatch"
            />
          </label>
          <label className="field">
            <span>Redirect URI</span>
            <input
              className="input"
              value={config.redirectUri}
              onChange={(e) => setConfig({ ...config, redirectUri: e.target.value })}
            />
            <small className="muted">
              Must match a registered redirect URI on the IdP's client
              entry. Default is this SPA's origin + <code>/oidc-callback</code>.
            </small>
          </label>
          <label className="field">
            <span>Scopes</span>
            <input
              className="input"
              value={config.scope ?? ""}
              onChange={(e) => setConfig({ ...config, scope: e.target.value })}
            />
          </label>
          <label className="field">
            <span>Storage URL (paired with this session)</span>
            <input
              className="input"
              value={config.storageUrl ?? ""}
              onChange={(e) => setConfig({ ...config, storageUrl: e.target.value })}
              placeholder="https://storage.ems-prague.cz"
            />
            <small className="muted">
              Optional — pre-fills Settings → Storage URL once signed in.
            </small>
          </label>
        </div>
      </section>

      <div className="panel-actions">
        <button
          type="button"
          className="btn btn-primary"
          disabled={!isConfigured || pending}
          onClick={handleSignIn}
        >
          {pending ? "Redirecting…" : "Sign in with operator IdP"}
        </button>
        {error && (
          <span className="error mono" style={{ color: "var(--danger, #b00020)", marginLeft: 12 }}>
            {error}
          </span>
        )}
      </div>

      <p className="muted footnote">
        Build-time defaults: set <code>VITE_OIDC_ISSUER</code>,{" "}
        <code>VITE_OIDC_CLIENT_ID</code>, <code>VITE_OIDC_REDIRECT_URI</code>,{" "}
        <code>VITE_OIDC_SCOPE</code>, and <code>VITE_STORAGE_URL</code> to skip
        this form on subsequent deployments. The paste-token UX on{" "}
        <a href="/settings">Settings</a> continues to work as a fallback for
        shifts when the IdP is unreachable.
      </p>
    </div>
  );
}
