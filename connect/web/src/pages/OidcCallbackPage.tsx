import { OidcCallbackPage as SharedOidcCallbackPage } from "@ohd/shared-web/OidcCallbackPage";
import { defaultOidcConfig, toSharedOptions } from "../ohdc/oidc";

/**
 * Receives the OIDC redirect (`/oidc-callback?code=...&state=...`),
 * exchanges the code for tokens via PKCE, and stashes the self-session
 * `ohds_…` token in sessionStorage. Then navigates to the log page.
 *
 * The actual callback machinery lives in `@ohd/shared-web` —
 * this component just feeds it the connect-flavoured config and the
 * `/log` success path.
 */
export function OidcCallbackPage() {
  return (
    <SharedOidcCallbackPage
      options={toSharedOptions(defaultOidcConfig())}
      successPath="/log"
    />
  );
}
