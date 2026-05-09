import { useState } from "react";
import { beginLogin, defaultOidcConfig, type OidcConfig } from "../ohdc/oidc";

/**
 * Operator sign-in page. Renders a "Sign in with the clinic SSO" button
 * (or a config form if VITE_OIDC_* env vars are missing). The actual
 * flow is OAuth 2.0 Authorization Code + PKCE per
 * `spec/docs/design/care-auth.md`.
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
    <div className="empty">
      <h2>Sign in to OHD Care</h2>
      <p className="muted">
        OHD Care authenticates clinicians via the clinic's OIDC provider
        (Hospital ADFS / Entra, Google Workspace, Authentik, Keycloak, …).
        Configure the issuer + client_id and we'll run the OAuth Code +
        PKCE flow.
      </p>

      <div className="form" style={{ maxWidth: 480 }}>
        <label>
          OIDC issuer URL
          <input
            value={config.issuer}
            onChange={(e) => setConfig({ ...config, issuer: e.target.value })}
            placeholder="https://login.microsoftonline.com/<tenant>/v2.0"
          />
        </label>
        <label>
          OAuth client_id
          <input
            value={config.clientId}
            onChange={(e) => setConfig({ ...config, clientId: e.target.value })}
            placeholder="<client_id>"
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
          disabled={!isConfigured || pending}
          onClick={handleSignIn}
        >
          {pending ? "Redirecting…" : "Sign in with the clinic SSO"}
        </button>
        {error && (
          <p className="error" style={{ color: "var(--danger, #b00020)" }}>
            {error}
          </p>
        )}
        <p className="muted" style={{ fontSize: 12 }}>
          Set <code>VITE_OIDC_ISSUER</code>, <code>VITE_OIDC_CLIENT_ID</code>,{" "}
          <code>VITE_OIDC_REDIRECT_URI</code>, and <code>VITE_OIDC_SCOPE</code> at
          build time to skip this form. The flow uses PKCE; no client
          secret is needed for public clients.
        </p>
      </div>
    </div>
  );
}
