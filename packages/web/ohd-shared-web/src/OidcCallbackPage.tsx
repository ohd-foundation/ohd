// Generic OIDC callback page. Receives the redirect-back from the AS,
// runs the code exchange, and either navigates to `successPath` or
// surfaces the error in the existing per-SPA stylings ("empty" /
// "page > empty"). Each SPA picks its own success path and an optional
// `onSessionComplete` hook for SPA-specific side effects (e.g. dispatch
// stores the storage URL claimed by the operator IdP into its OHDC
// client config before navigating away).

import { useEffect, useState, type ReactNode } from "react";
import { useNavigate } from "react-router-dom";
import { completeLogin, type OidcOptions, type OidcSession } from "./oidc";

export interface OidcCallbackPageProps {
  /** OIDC engine options — same shape as the matching `oidc.ts` wrapper. */
  options: OidcOptions;
  /** Where to navigate after a successful exchange. */
  successPath: string;
  /**
   * Optional side-effect run after `completeLogin` resolves and before
   * navigation — e.g. dispatch setting the OHDC client's `storageUrl`.
   */
  onSessionComplete?: (session: OidcSession) => void | Promise<void>;
  /**
   * Renders the layout chrome around the success / error body. Defaults
   * to `<div className="empty">…</div>`. Dispatch passes a wrapper that
   * adds the `<div className="page">` shell around it.
   */
  layout?: (body: ReactNode) => ReactNode;
}

const defaultLayout = (body: ReactNode): ReactNode => (
  <div className="empty">{body}</div>
);

export function OidcCallbackPage(props: OidcCallbackPageProps) {
  const { options, successPath, onSessionComplete, layout = defaultLayout } = props;
  const [error, setError] = useState<string | null>(null);
  const navigate = useNavigate();

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const session = await completeLogin(options, {
          search: window.location.search,
          redirectUri: options.redirectUri,
        });
        if (cancelled) return;
        if (onSessionComplete) {
          await onSessionComplete(session);
          if (cancelled) return;
        }
        navigate(successPath, { replace: true });
      } catch (err) {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : String(err));
        }
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [navigate, onSessionComplete, options, successPath]);

  if (error) {
    return (
      <>
        {layout(
          <>
            <h2>Sign-in failed</h2>
            <p className="error mono" style={{ fontSize: 12 }}>
              {error}
            </p>
            <p>
              <a href="/login">Try again</a>
            </p>
          </>
        )}
      </>
    );
  }
  return <>{layout(<p>Completing sign-in…</p>)}</>;
}
