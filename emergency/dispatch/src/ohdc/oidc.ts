// Operator OIDC sign-in for the OHD Emergency dispatch console.
//
// Per `spec/docs/design/auth.md` "Operator authentication", the
// dispatcher signs in against the operator's IdP — Keycloak / Authentik
// / Auth0 / Azure AD — via OAuth 2.0 Authorization Code + PKCE. The
// resulting bearer is stored in localStorage (operator hardware,
// longer-lived than the patient-side connect/web sessionStorage) under
// the existing key the rest of the dispatch SPA already reads.
//
// The actual flow lives in `@ohd/shared-web/oidc` — this module is a
// thin Dispatch-flavoured wrapper that:
//   - keeps the existing `OperatorSession` shape (with `operatorName` /
//     `subject`) so call sites stay unchanged,
//   - mirrors the access token via `setOperatorToken()` so `client.ts`
//     (which reads `ohd-dispatch-operator-token` directly) picks it
//     up,
//   - selects the dispatch-specific defaults (storage namespace, scope,
//     oauth2-then-oidc discovery, localStorage backing,
//     unsafe-decode id_token claim extraction).

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
import { setOperatorToken } from "./client";

export type { CallbackParams };

/** Persisted operator-session payload. */
export interface OperatorSession {
  /** Bearer token used as `Authorization: Bearer …` on every OHDC call. */
  accessToken: string;
  /** Optional refresh token. */
  refreshToken?: string;
  /** Unix ms when the access token stops being valid. */
  expiresAtMs?: number;
  /** Issuer URL the bearer was minted by. */
  issuer?: string;
  /** Storage URL targeted by this session. */
  storageUrl?: string;
  /** Operator's display name from the OIDC `name` / `preferred_username` claim. */
  operatorName?: string;
  /** OIDC `sub` claim, when known. */
  subject?: string;
}

export interface OidcConfig {
  /** OIDC issuer URL — the operator's IdP. */
  issuer: string;
  /** OAuth client_id registered with the IdP for this SPA. */
  clientId: string;
  /** Where the IdP sends the user back. Default `${origin}/oidc-callback`. */
  redirectUri: string;
  /** OAuth scopes. Default `openid profile offline_access`. */
  scope?: string;
  /** Storage URL the operator-session token will be paired with. */
  storageUrl?: string;
}

/** Default config from VITE_* env vars + window.origin. */
export function defaultOidcConfig(): OidcConfig {
  const env = (typeof import.meta !== "undefined" ? import.meta.env : undefined) as
    | Record<string, string | undefined>
    | undefined;
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  return {
    issuer: env?.VITE_OIDC_ISSUER ?? "",
    clientId: env?.VITE_OIDC_CLIENT_ID ?? "ohd-emergency-dispatch",
    redirectUri: env?.VITE_OIDC_REDIRECT_URI ?? `${origin}/oidc-callback`,
    scope: env?.VITE_OIDC_SCOPE ?? "openid profile offline_access",
    storageUrl: env?.VITE_STORAGE_URL ?? "",
  };
}

// ---------------------------------------------------------------------------
// Dispatch-flavoured options for the shared engine
// ---------------------------------------------------------------------------

export function toSharedOptions(config: OidcConfig): OidcOptions {
  return {
    issuer: config.issuer,
    clientId: config.clientId,
    redirectUri: config.redirectUri,
    scope: config.scope ?? "openid profile offline_access",
    storageUrl: config.storageUrl,
    discoveryAlgorithm: "oauth2-then-oidc",
    sessionStorageBackend: "local",
    storageNamespace: "ohd-dispatch-operator",
    idTokenClaims: "unsafe-decode",
    onSessionSaved: (s) => {
      // Mirror the access token under the legacy storage key so the
      // OHDC client (which reads `ohd-dispatch-operator-token`
      // directly) picks it up unchanged.
      setOperatorToken(s.accessToken);
    },
  };
}

function toOperator(s: OidcSession): OperatorSession {
  return {
    accessToken: s.accessToken,
    refreshToken: s.refreshToken,
    expiresAtMs: s.expiresAtMs,
    issuer: s.issuer,
    storageUrl: s.storageUrl,
    operatorName: s.displayName,
    subject: s.subject,
  };
}

function toEngine(s: OperatorSession): OidcSession {
  return {
    accessToken: s.accessToken,
    refreshToken: s.refreshToken,
    expiresAtMs: s.expiresAtMs,
    issuer: s.issuer,
    storageUrl: s.storageUrl,
    displayName: s.operatorName,
    subject: s.subject,
  };
}

// ---------------------------------------------------------------------------
// Public surface — same names + shapes as before
// ---------------------------------------------------------------------------

export async function beginLogin(config: OidcConfig): Promise<never> {
  if (!config.issuer) {
    throw new Error(
      "OIDC issuer not configured. Set VITE_OIDC_ISSUER or fill in the Sign-in form."
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

/** Read the persisted operator's display name (from id_token claims). */
export function operatorDisplayName(): string | null {
  return loadSession()?.operatorName ?? null;
}

export async function refreshIfNeeded(bufferMs = 60_000): Promise<OperatorSession | null> {
  const s = await refreshIfNeededShared(toSharedOptions(defaultOidcConfig()), bufferMs);
  return s ? toOperator(s) : null;
}
