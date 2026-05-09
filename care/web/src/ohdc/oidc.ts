// OIDC / OAuth 2.0 Authorization Code Flow with PKCE for OHD Care web.
//
// Per `spec/docs/design/care-auth.md` "Operator authentication into Care",
// the clinician logs into Care via the clinic's OIDC SSO before the app
// dispatches any OHDC calls against patient grants.
//
// The actual flow lives in `@ohd/shared-web/oidc` — this module is a
// thin Care-flavoured wrapper that:
//   - keeps the existing `OperatorSession` shape (with `oidcSubject` /
//     `oidcIssuer` rather than the engine's `subject` / `issuer`) so
//     the rest of the SPA (e.g. `client.ts` reading
//     `session.oidcSubject` for the operator-subject header) stays
//     unchanged,
//   - selects the care-specific defaults (storage namespace, scope,
//     pure OIDC discovery, validated id_token claim extraction).

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

/** Persisted operator session payload after a successful /token exchange. */
export interface OperatorSession {
  accessToken: string;
  refreshToken?: string;
  /** Unix ms when the access token stops being valid (best-effort). */
  expiresAtMs?: number;
  /** The `sub` claim of the upstream OIDC `id_token`. */
  oidcSubject?: string;
  /** The `iss` claim of the issuer that minted the session. */
  oidcIssuer?: string;
  /** Optional display claims (name, preferred_username, email). */
  displayName?: string;
  email?: string;
}

export interface OidcConfig {
  issuer: string;       // e.g. "https://login.microsoftonline.com/<tenant>/v2.0"
  clientId: string;     // OAuth client_id registered with the issuer
  redirectUri: string;  // typically `${origin}/oidc-callback`
  scope?: string;       // default "openid profile email offline_access"
}

/**
 * Build an :class:`OidcConfig` from VITE_* env vars, falling back to
 * empty values that surface in the login UI as "configure me".
 */
export function defaultOidcConfig(): OidcConfig {
  const env = (typeof import.meta !== "undefined" ? import.meta.env : undefined) as
    | Record<string, string | undefined>
    | undefined;
  const origin =
    typeof window !== "undefined" ? window.location.origin : "http://localhost:5173";
  return {
    issuer: env?.VITE_OIDC_ISSUER ?? "",
    clientId: env?.VITE_OIDC_CLIENT_ID ?? "",
    redirectUri: env?.VITE_OIDC_REDIRECT_URI ?? `${origin}/oidc-callback`,
    scope: env?.VITE_OIDC_SCOPE ?? "openid profile email offline_access",
  };
}

// ---------------------------------------------------------------------------
// Care-flavoured options for the shared engine
// ---------------------------------------------------------------------------

export function toSharedOptions(config: OidcConfig): OidcOptions {
  return {
    issuer: config.issuer,
    clientId: config.clientId,
    redirectUri: config.redirectUri,
    scope: config.scope ?? "openid profile email offline_access",
    discoveryAlgorithm: "oidc",
    sessionStorageBackend: "session",
    storageNamespace: "ohd-care-operator",
    idTokenClaims: "validated",
  };
}

function toOperator(s: OidcSession): OperatorSession {
  return {
    accessToken: s.accessToken,
    refreshToken: s.refreshToken,
    expiresAtMs: s.expiresAtMs,
    oidcSubject: s.subject,
    oidcIssuer: s.issuer,
    displayName: s.displayName,
    email: s.email,
  };
}

function toEngine(s: OperatorSession): OidcSession {
  return {
    accessToken: s.accessToken,
    refreshToken: s.refreshToken,
    expiresAtMs: s.expiresAtMs,
    issuer: s.oidcIssuer,
    subject: s.oidcSubject,
    displayName: s.displayName,
    email: s.email,
  };
}

// ---------------------------------------------------------------------------
// Public surface — same names + shapes as before
// ---------------------------------------------------------------------------

export async function beginLogin(config: OidcConfig): Promise<never> {
  if (!config.issuer || !config.clientId) {
    throw new Error(
      "OIDC issuer / client_id not configured. Set VITE_OIDC_ISSUER and " +
        "VITE_OIDC_CLIENT_ID at build time, or paste them on the Sign-in screen."
    );
  }
  return await beginLoginShared(toSharedOptions(config));
}

export async function completeLogin(params: CallbackParams): Promise<OperatorSession> {
  const session = await completeLoginShared(toSharedOptions(defaultOidcConfig()), params);
  return toOperator(session);
}

export function saveSession(session: OperatorSession): void {
  saveSessionShared(toSharedOptions(defaultOidcConfig()), toEngine(session));
}

export function loadSession(): OperatorSession | null {
  const s = loadSessionShared(toSharedOptions(defaultOidcConfig()));
  return s ? toOperator(s) : null;
}

export function clearSession(): void {
  clearSessionShared(toSharedOptions(defaultOidcConfig()));
}

export async function refreshIfNeeded(bufferMs = 60_000): Promise<OperatorSession | null> {
  const s = await refreshIfNeededShared(toSharedOptions(defaultOidcConfig()), bufferMs);
  return s ? toOperator(s) : null;
}
