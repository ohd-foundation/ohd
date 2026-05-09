import type { ReactNode } from "react";
import { OidcCallbackPage as SharedOidcCallbackPage } from "@ohd/shared-web/OidcCallbackPage";
import { defaultOidcConfig, toSharedOptions } from "../ohdc/oidc";
import { setStorageUrl } from "../ohdc/client";

/**
 * Receives the OIDC redirect (`/oidc-callback?code=...&state=...`),
 * exchanges the code for tokens via PKCE, and stashes the operator
 * session bearer under the existing `ohd-dispatch-operator-token` key
 * so the rest of the SPA's OHDC client picks it up. Then routes to
 * `/active`.
 *
 * Wraps the body in dispatch's `<div className="page">` shell so the
 * app shell layout matches the rest of the SPA.
 */
const dispatchLayout = (body: ReactNode): ReactNode => (
  <div className="page">
    <div className="empty">{body}</div>
  </div>
);

export function OidcCallbackPage() {
  return (
    <SharedOidcCallbackPage
      options={toSharedOptions(defaultOidcConfig())}
      successPath="/active"
      onSessionComplete={(session) => {
        if (session.storageUrl) {
          setStorageUrl(session.storageUrl);
        }
      }}
      layout={dispatchLayout}
    />
  );
}
