import { useEffect, useState } from "react";
import { api, ApiError, type AuthProvider } from "../api";
import { ErrorBanner, Spinner } from "../components/common";

export default function LoginPage() {
  const [providers, setProviders] = useState<AuthProvider[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api
      .authProviders()
      .then((d) => setProviders(d.providers))
      .catch((e) =>
        setError(
          e instanceof ApiError ? e.message : "Could not reach the server",
        ),
      );
  }, []);

  if (!providers && !error) return <Spinner label="Loading sign-in" />;

  // Real identity providers (kind=oidc) become full-width buttons; the
  // dev-bypass provider (kind=dev) becomes a small corner link.
  const real = providers?.filter((p) => p.kind !== "dev") ?? [];
  const dev = providers?.find((p) => p.kind === "dev") ?? null;

  return (
    <div className="center-screen">
      <div className="card login-card">
        <img
          className="login-logo"
          src="/brand/cord-on-white.svg"
          alt="OHD CORD"
          width={72}
          height={72}
        />
        <h1>OHD CORD</h1>
        <p className="muted">Sign in to talk to your health data.</p>

        {error && (
          <div style={{ marginTop: 20 }}>
            <ErrorBanner message={error} />
          </div>
        )}

        {providers && real.length === 0 && !dev && (
          <div className="banner info" style={{ marginTop: 20 }}>
            No sign-in providers are configured on this deployment. Login is
            unavailable.
          </div>
        )}

        {real.length > 0 && (
          <div className="provider-list">
            {real.map((p) => (
              <OidcButton key={p.id} provider={p} />
            ))}
          </div>
        )}

        {dev && (
          <div className="login-dev-bypass">
            <a
              href={api.authStartUrl(dev.id)}
              onClick={(e) => {
                e.preventDefault();
                window.location.href = api.authStartUrl(dev.id);
              }}
            >
              → demo
            </a>
          </div>
        )}
      </div>
    </div>
  );
}

/**
 * Single OIDC provider button, styled like a real social-login button
 * (white surface, framed, brand wordmark prominently displayed). The
 * `id === "ohd"` branch renders the OHD Identity wordmark with the
 * project's red+black split so it visually matches the rest of the
 * brand surface; other providers fall back to a generic label.
 */
function OidcButton({ provider }: { provider: AuthProvider }) {
  const go = () => {
    // window.location.href, not fetch — the IdP responds with a 302.
    window.location.href = api.authStartUrl(provider.id);
  };
  return (
    <button className="oidc-button" onClick={go}>
      {provider.id === "ohd" ? (
        <span className="oidc-button-label">
          Continue with{" "}
          <span className="oidc-brand">
            <span className="oidc-brand-ohd">OHD</span>{" "}
            <span className="oidc-brand-identity">Identity</span>
          </span>
        </span>
      ) : (
        <span className="oidc-button-label">
          Continue with{" "}
          <span className="oidc-brand">{capitalize(provider.id)}</span>
        </span>
      )}
      <span className="oidc-issuer">{provider.issuer}</span>
    </button>
  );
}

function capitalize(s: string) {
  return s.length ? s[0].toUpperCase() + s.slice(1) : s;
}
