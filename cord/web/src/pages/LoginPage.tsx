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

  return (
    <div className="center-screen">
      <div className="card login-card">
        <h1>OHD CORD</h1>
        <p className="muted">Sign in to talk to your health data.</p>

        {error && (
          <div style={{ marginTop: 20 }}>
            <ErrorBanner message={error} />
          </div>
        )}

        {providers && providers.length === 0 && (
          <div className="banner info" style={{ marginTop: 20 }}>
            No sign-in providers are configured on this deployment. Login is
            unavailable (dev mode).
          </div>
        )}

        {providers && providers.length > 0 && (
          <div className="provider-list">
            {providers.map((p) => (
              <button
                key={p.id}
                className="primary"
                onClick={() => {
                  // Must navigate, not fetch: this 302s through the IdP.
                  window.location.href = api.authStartUrl(p.id);
                }}
              >
                <div>Continue with {p.id}</div>
                <div className="provider-issuer">{p.issuer}</div>
              </button>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
