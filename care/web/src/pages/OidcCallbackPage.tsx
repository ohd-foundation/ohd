import { OidcCallbackPage as SharedOidcCallbackPage } from "@ohd/shared-web/OidcCallbackPage";
import { defaultOidcConfig, toSharedOptions } from "../ohdc/oidc";

/**
 * Receives the OIDC redirect (`/oidc-callback?code=...&state=...`),
 * exchanges the code for tokens via PKCE, and stashes the operator
 * session in sessionStorage. Then navigates to the roster.
 */
export function OidcCallbackPage() {
  return (
    <SharedOidcCallbackPage
      options={toSharedOptions(defaultOidcConfig())}
      successPath="/roster"
    />
  );
}
