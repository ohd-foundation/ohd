// Self-session OIDC sign-in for OHD Connect web.
//
// Per `spec/docs/design/auth.md` "Browser-based clients (Connect mobile,
// Connect web, Care web)", OHD Storage acts as the OAuth 2.0
// Authorization Server toward the SPA. The user's `?token=ohds_...`
// paste-token UX is preserved as a fast path; this module adds the
// real OAuth Code + PKCE flow against `<storage>/authorize` +
// `<storage>/token`.
//
// The actual flow lives in `@ohd/shared-web/oidc` — this module is a
// thin Connect-flavoured wrapper that:
//   - keeps the existing `OidcConfig` shape (with `storageUrl` rather
//     than the shared engine's `issuer`) so call sites in `LoginPage`
//     and the rest of the SPA stay unchanged,
//   - mirrors the access token under `SELF_TOKEN_STORAGE_KEY` for the
//     client-side bearer reader in `client.ts`,
//   - selects the connect-specific defaults (storage namespace, scope,
//     fallback discovery, sessionStorage backing).

import {
  beginLogin as beginLoginShared,
  completeLogin as completeLoginShared,
  loadSession as loadSessionShared,
  saveSession as saveSessionShared,
  clearSession as clearSessionShared,
  refreshIfNeeded as refreshIfNeededShared,
  type CallbackParams,
  type OidcOptions,
  type OidcSession,
} from "@ohd/shared-web/oidc";

export type { CallbackParams };

// Re-export this so client.ts can read the bearer token without
// importing oauth.* itself.
export const SELF_TOKEN_STORAGE_KEY = "ohd-connect-self-token";

/** Persisted self-session payload. */
export interface SelfSession {
  /** The opaque `ohds_…` access token. */
  accessToken: string;
  /** The opaque `ohdr_…` refresh token, if issued. */
  refreshToken?: string;
  /** Unix ms when the access token stops being valid. */
  expiresAtMs?: number;
  /** Issuer URL (the storage AS) that minted the session. */
  issuer?: string;
  /** Storage URL targeted by this session. */
  storageUrl?: string;
}

export interface OidcConfig {
  /** Storage URL acting as OAuth AS (e.g. `https://ohd.cloud.example`). */
  storageUrl: string;
  /** OAuth client_id (for public clients, often the SPA's origin). */
  clientId: string;
  /** Where the AS sends the user back. Default `${origin}/oidc-callback`. */
  redirectUri: string;
  /** OAuth scopes. Default `openid offline_access`. */
  scope?: string;
}

/** Default config from VITE_* env vars + window.origin. */
export function defaultOidcConfig(): OidcConfig {
  const env = (typeof import.meta !== "undefined" ? import.meta.env : undefined) as
    | Record<string, string | undefined>
    | undefined;
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  return {
    storageUrl: env?.VITE_OIDC_STORAGE_URL ?? env?.VITE_STORAGE_URL ?? "",
    clientId: env?.VITE_OIDC_CLIENT_ID ?? "ohd-connect-web",
    redirectUri: env?.VITE_OIDC_REDIRECT_URI ?? `${origin}/oidc-callback`,
    scope: env?.VITE_OIDC_SCOPE ?? "openid offline_access",
  };
}

// ---------------------------------------------------------------------------
// Connect-flavoured options for the shared engine
// ---------------------------------------------------------------------------

/**
 * Catalog key for OHD's first-party OIDC provider (the `ohd-idp` service at
 * `accounts.ohd.dev`). Passing this to {@link beginLogin} deep-links the
 * storage AS straight to "Sign in with OHD".
 */
export const OHD_ACCOUNT_PROVIDER = "ohd_account";

/**
 * Build the shared-engine options.
 *
 * `providerHint` (optional) is forwarded to the storage AS as
 * `?provider=<key>` so the AS skips its provider-picker page and delegates
 * straight to that upstream OIDC provider.
 */
export function toSharedOptions(
  config: OidcConfig,
  providerHint?: string
): OidcOptions {
  return {
    issuer: config.storageUrl,
    clientId: config.clientId,
    redirectUri: config.redirectUri,
    scope: config.scope ?? "openid offline_access",
    discoveryAlgorithm: "oauth2-then-fallback-paths",
    sessionStorageBackend: "session",
    storageNamespace: "ohd-connect",
    idTokenClaims: "skip",
    extraAuthorizeParams: providerHint ? { provider: providerHint } : undefined,
    onSessionSaved: (s) => {
      // Mirror the access token under the legacy storage key so existing
      // call sites in `client.ts` continue to read it.
      sessionStorage.setItem(SELF_TOKEN_STORAGE_KEY, s.accessToken);
    },
    onSessionCleared: () => {
      sessionStorage.removeItem(SELF_TOKEN_STORAGE_KEY);
    },
  };
}

function toSelf(s: OidcSession): SelfSession {
  return {
    accessToken: s.accessToken,
    refreshToken: s.refreshToken,
    expiresAtMs: s.expiresAtMs,
    issuer: s.issuer,
    storageUrl: s.storageUrl ?? s.issuer,
  };
}

// ---------------------------------------------------------------------------
// Public surface — same names + shapes as before
// ---------------------------------------------------------------------------

export async function beginLogin(
  config: OidcConfig,
  providerHint?: string
): Promise<never> {
  if (!config.storageUrl) {
    throw new Error(
      "OIDC storage URL not configured. Set VITE_OIDC_STORAGE_URL or " +
        "fill in the Sign-in form."
    );
  }
  return await beginLoginShared(toSharedOptions(config, providerHint));
}

export async function completeLogin(params: CallbackParams): Promise<SelfSession> {
  // The defaults are fine — only the storageNamespace + storage backend
  // matter for the callback (issuer/clientId are read out of
  // sessionStorage by the shared engine).
  const session = await completeLoginShared(toSharedOptions(defaultOidcConfig()), params);
  return toSelf(session);
}

export function saveSession(session: SelfSession): void {
  saveSessionShared(toSharedOptions(defaultOidcConfig()), session);
}

export function loadSession(): SelfSession | null {
  const s = loadSessionShared(toSharedOptions(defaultOidcConfig()));
  return s ? toSelf(s) : null;
}

export function clearSession(): void {
  clearSessionShared(toSharedOptions(defaultOidcConfig()));
}

export async function refreshIfNeeded(bufferMs = 60_000): Promise<SelfSession | null> {
  const s = await refreshIfNeededShared(toSharedOptions(defaultOidcConfig()), bufferMs);
  return s ? toSelf(s) : null;
}
