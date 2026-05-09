// Generic OAuth 2.0 Authorization Code + PKCE flow against any OAuth/OIDC
// issuer, factored out of the three OHD SPAs (connect/web, care/web,
// emergency/dispatch). Each SPA wraps this with its own thin
// configuration shim so call sites stay unchanged.
//
// The flow shape is RFC 6749 + RFC 7636 + RFC 8414, identical across all
// three callers. The shared engine accepts an :class:`OidcOptions` that
// captures the points of legitimate divergence:
//
//   - *Discovery algorithm* — connect/web treats storage as an OAuth-AS
//     (oauth2 metadata + a hard-coded fallback), care/web speaks pure
//     OIDC, emergency/dispatch tries oauth2-then-oidc.
//   - *Session storage backing* — sessionStorage (connect, care) vs
//     localStorage (emergency, since the dispatcher's machine is shared
//     hardware that survives tab close).
//   - *Storage namespace* — each SPA stamps its keys with its own prefix
//     so a single browser hosting two SPAs at once doesn't collide.
//   - *Id-token claim extraction* — care uses the validated path that
//     oauth4webapi exposes; emergency does an unsafe base64url-decode
//     because its IdP doesn't always sign the id_token in the way
//     oauth4webapi requires for the strict path.
//   - *Side-effect hooks* — connect mirrors the access token under a
//     legacy storage key; emergency calls a `setOperatorToken` setter
//     in its OHDC client. Both happen via callbacks so the engine
//     itself stays decoupled.
//
// What is intentionally not configurable: the cryptographic primitives
// (PKCE S256, random state), the error-on-missing-state behaviour, and
// the wire shape of the OAuth requests themselves. Those are the
// protocol — not policy.

import * as oauth from "oauth4webapi";

/** How to discover the AS metadata. */
export type DiscoveryAlgorithm =
  /** OAuth 2.0 metadata (`/.well-known/oauth-authorization-server`, RFC 8414). */
  | "oauth2"
  /** OIDC discovery (`/.well-known/openid-configuration`). */
  | "oidc"
  /** Try OAuth 2.0 metadata first, fall back to OIDC discovery on 404. */
  | "oauth2-then-oidc"
  /**
   * Try OAuth 2.0 metadata first, fall back to a hard-coded set of
   * endpoints synthesised from the issuer URL on failure. Used when the
   * AS doesn't ship a discovery doc yet but its endpoints follow the
   * conventional `/authorize` + `/token` paths.
   */
  | "oauth2-then-fallback-paths";

/** Persisted session payload, common shape across all SPAs. */
export interface OidcSession {
  accessToken: string;
  refreshToken?: string;
  /** Unix ms when the access token stops being valid (best-effort). */
  expiresAtMs?: number;
  /** Issuer URL that minted the session. */
  issuer?: string;
  /** Optional storage URL paired with this session (emergency). */
  storageUrl?: string;
  /** OIDC `sub` claim, when known. */
  subject?: string;
  /** Operator's display name (`name` or `preferred_username` claim). */
  displayName?: string;
  /** Email claim from the id_token, when present. */
  email?: string;
}

export interface OidcOptions {
  /** Issuer / AS root URL. */
  issuer: string;
  /** OAuth public client_id. */
  clientId: string;
  /** Redirect URI registered with the AS. */
  redirectUri: string;
  /** OAuth scope string (space-separated). Caller decides defaults. */
  scope: string;
  /** How to discover the AS metadata. */
  discoveryAlgorithm: DiscoveryAlgorithm;
  /** Where the persisted session lives. */
  sessionStorageBackend: "session" | "local";
  /** Prefix for all storage keys (`ohd-connect`, `ohd-care`, `ohd-dispatch`). */
  storageNamespace: string;
  /** How to read the id_token claims after the code exchange. */
  idTokenClaims: "validated" | "unsafe-decode" | "skip";
  /** Optional storage URL the session is paired with (emergency). */
  storageUrl?: string;
  /**
   * Called after a session is persisted (initial login or refresh).
   * Used by connect and emergency to mirror the access token under a
   * legacy storage key the rest of the SPA reads.
   */
  onSessionSaved?: (session: OidcSession) => void;
  /** Called by `clearSession` so the legacy mirror can be wiped too. */
  onSessionCleared?: () => void;
}

// ---------------------------------------------------------------------------
// Storage-key helpers
// ---------------------------------------------------------------------------

interface StorageKeys {
  pkceVerifier: string;
  pkceState: string;
  asUrl: string;
  clientId: string;
  session: string;
  pendingStorageUrl: string;
}

function keys(ns: string): StorageKeys {
  return {
    pkceVerifier: `${ns}-pkce-verifier`,
    pkceState: `${ns}-pkce-state`,
    asUrl: `${ns}-as-url`,
    clientId: `${ns}-as-client-id`,
    session: `${ns}-session`,
    pendingStorageUrl: `${ns}-pending-storage-url`,
  };
}

function sessionBackend(opts: OidcOptions): Storage {
  // PKCE verifier, state, and the in-flight AS URL must round-trip
  // across the redirect — sessionStorage suffices and is wiped on close.
  // We always use sessionStorage for these (even when the persistent
  // session lives in localStorage), matching the existing per-SPA
  // behaviour.
  void opts;
  return sessionStorage;
}

function persistBackend(opts: OidcOptions): Storage {
  return opts.sessionStorageBackend === "local" ? localStorage : sessionStorage;
}

// ---------------------------------------------------------------------------
// AS discovery
// ---------------------------------------------------------------------------

async function discover(
  issuer: string,
  algorithm: DiscoveryAlgorithm
): Promise<oauth.AuthorizationServer> {
  const url = new URL(issuer);
  if (algorithm === "oidc") {
    const response = await oauth.discoveryRequest(url, { algorithm: "oidc" });
    return await oauth.processDiscoveryResponse(url, response);
  }
  if (algorithm === "oauth2") {
    const response = await oauth.discoveryRequest(url, { algorithm: "oauth2" });
    return await oauth.processDiscoveryResponse(url, response);
  }
  if (algorithm === "oauth2-then-oidc") {
    try {
      const response = await oauth.discoveryRequest(url, { algorithm: "oauth2" });
      return await oauth.processDiscoveryResponse(url, response);
    } catch {
      const response = await oauth.discoveryRequest(url, { algorithm: "oidc" });
      return await oauth.processDiscoveryResponse(url, response);
    }
  }
  // oauth2-then-fallback-paths: synthesise a metadata object from
  // convention. The fallback matches storage's documented OAuth surface
  // per `spec/docs/design/auth.md`.
  try {
    const response = await oauth.discoveryRequest(url, { algorithm: "oauth2" });
    return await oauth.processDiscoveryResponse(url, response);
  } catch {
    const base = issuer.replace(/\/$/, "");
    return {
      issuer: base,
      authorization_endpoint: `${base}/authorize`,
      token_endpoint: `${base}/token`,
      device_authorization_endpoint: `${base}/device`,
      registration_endpoint: `${base}/oauth/register`,
      code_challenge_methods_supported: ["S256"],
      grant_types_supported: [
        "authorization_code",
        "refresh_token",
        "urn:ietf:params:oauth:grant-type:device_code",
      ],
      response_types_supported: ["code"],
    } as oauth.AuthorizationServer;
  }
}

// ---------------------------------------------------------------------------
// Flow entry
// ---------------------------------------------------------------------------

/**
 * Kick off the Authorization Code + PKCE flow. After this resolves the
 * page has navigated to the issuer's `/authorize` endpoint — the caller
 * never sees the resolution.
 */
export async function beginLogin(opts: OidcOptions): Promise<never> {
  if (!opts.issuer) {
    throw new Error(
      "OIDC issuer not configured. Set the env var or fill in the Sign-in form."
    );
  }
  const as = await discover(opts.issuer, opts.discoveryAlgorithm);
  if (!as.authorization_endpoint) {
    throw new Error(
      "AS metadata missing `authorization_endpoint`. Issuer may not be a compliant OAuth/OIDC server."
    );
  }
  const codeVerifier = oauth.generateRandomCodeVerifier();
  const codeChallenge = await oauth.calculatePKCECodeChallenge(codeVerifier);
  const state = oauth.generateRandomState();

  const k = keys(opts.storageNamespace);
  const ss = sessionBackend(opts);
  ss.setItem(k.pkceVerifier, codeVerifier);
  ss.setItem(k.pkceState, state);
  ss.setItem(k.asUrl, opts.issuer);
  ss.setItem(k.clientId, opts.clientId);
  if (opts.storageUrl) {
    ss.setItem(k.pendingStorageUrl, opts.storageUrl);
  }

  const url = new URL(as.authorization_endpoint);
  url.searchParams.set("response_type", "code");
  url.searchParams.set("client_id", opts.clientId);
  url.searchParams.set("redirect_uri", opts.redirectUri);
  url.searchParams.set("scope", opts.scope);
  url.searchParams.set("code_challenge", codeChallenge);
  url.searchParams.set("code_challenge_method", "S256");
  url.searchParams.set("state", state);

  window.location.assign(url.toString());
  // Browser is leaving — satisfy the `never` return type.
  return await new Promise<never>(() => {});
}

// ---------------------------------------------------------------------------
// Callback handling
// ---------------------------------------------------------------------------

export interface CallbackParams {
  /** Full window.location.search string. */
  search: string;
  redirectUri: string;
}

export async function completeLogin(
  opts: OidcOptions,
  params: CallbackParams
): Promise<OidcSession> {
  const k = keys(opts.storageNamespace);
  const ss = sessionBackend(opts);
  const issuer = ss.getItem(k.asUrl);
  const clientId = ss.getItem(k.clientId);
  const codeVerifier = ss.getItem(k.pkceVerifier);
  const expectedState = ss.getItem(k.pkceState);
  const pendingStorageUrl = ss.getItem(k.pendingStorageUrl) || undefined;

  if (!issuer || !clientId || !codeVerifier || !expectedState) {
    throw new Error(
      "OIDC callback hit without an in-flight login (missing state / verifier)."
    );
  }

  const as = await discover(issuer, opts.discoveryAlgorithm);
  const client: oauth.Client = { client_id: clientId, token_endpoint_auth_method: "none" };
  const clientAuth = oauth.None();

  const callbackParams = oauth.validateAuthResponse(
    as,
    client,
    new URLSearchParams(params.search),
    expectedState
  );
  const tokenResponse = await oauth.authorizationCodeGrantRequest(
    as,
    client,
    clientAuth,
    callbackParams,
    params.redirectUri,
    codeVerifier
  );
  const tokenJson = await oauth.processAuthorizationCodeResponse(as, client, tokenResponse);

  const expiresIn = typeof tokenJson.expires_in === "number" ? tokenJson.expires_in : undefined;

  // Pull id_token claims by the configured strategy.
  let subject: string | undefined;
  let displayName: string | undefined;
  let email: string | undefined;
  if (opts.idTokenClaims === "validated") {
    const claims = oauth.getValidatedIdTokenClaims(tokenJson);
    if (claims) {
      subject = typeof claims.sub === "string" ? claims.sub : undefined;
      displayName =
        typeof claims["name"] === "string"
          ? (claims["name"] as string)
          : typeof claims["preferred_username"] === "string"
            ? (claims["preferred_username"] as string)
            : undefined;
      email = typeof claims["email"] === "string" ? (claims["email"] as string) : undefined;
    }
  } else if (opts.idTokenClaims === "unsafe-decode") {
    const idToken = (tokenJson as { id_token?: string }).id_token;
    if (idToken) {
      try {
        const claims = decodeJwtClaims(idToken);
        subject = claims.sub as string | undefined;
        displayName =
          (claims.name as string | undefined) ?? (claims.preferred_username as string | undefined);
        email = claims.email as string | undefined;
      } catch {
        /* swallow — caller is on a best-effort path */
      }
    }
  }

  const session: OidcSession = {
    accessToken: tokenJson.access_token,
    refreshToken: tokenJson.refresh_token,
    expiresAtMs: expiresIn ? Date.now() + expiresIn * 1000 : undefined,
    issuer,
    storageUrl: pendingStorageUrl ?? opts.storageUrl,
    subject,
    displayName,
    email,
  };

  saveSession(opts, session);

  ss.removeItem(k.pkceVerifier);
  ss.removeItem(k.pkceState);
  ss.removeItem(k.pendingStorageUrl);
  return session;
}

// ---------------------------------------------------------------------------
// Session persistence
// ---------------------------------------------------------------------------

export function saveSession(opts: OidcOptions, session: OidcSession): void {
  const k = keys(opts.storageNamespace);
  persistBackend(opts).setItem(k.session, JSON.stringify(session));
  opts.onSessionSaved?.(session);
}

export function loadSession(opts: OidcOptions): OidcSession | null {
  const k = keys(opts.storageNamespace);
  const raw = persistBackend(opts).getItem(k.session);
  if (!raw) return null;
  try {
    const obj = JSON.parse(raw) as OidcSession;
    if (typeof obj.accessToken !== "string" || obj.accessToken.length === 0) return null;
    return obj;
  } catch {
    return null;
  }
}

export function clearSession(opts: OidcOptions): void {
  const k = keys(opts.storageNamespace);
  persistBackend(opts).removeItem(k.session);
  const ss = sessionBackend(opts);
  ss.removeItem(k.asUrl);
  ss.removeItem(k.clientId);
  opts.onSessionCleared?.();
}

// ---------------------------------------------------------------------------
// Silent refresh
// ---------------------------------------------------------------------------

export async function refreshIfNeeded(
  opts: OidcOptions,
  bufferMs = 60_000
): Promise<OidcSession | null> {
  const session = loadSession(opts);
  if (!session) return null;
  if (!session.expiresAtMs) return session;
  if (session.expiresAtMs - Date.now() > bufferMs) return session;
  if (!session.refreshToken || !session.issuer) return session;

  const k = keys(opts.storageNamespace);
  const clientId = sessionBackend(opts).getItem(k.clientId);
  if (!clientId) return session;
  try {
    const as = await discover(session.issuer, opts.discoveryAlgorithm);
    const client: oauth.Client = { client_id: clientId, token_endpoint_auth_method: "none" };
    const clientAuth = oauth.None();
    const response = await oauth.refreshTokenGrantRequest(
      as,
      client,
      clientAuth,
      session.refreshToken
    );
    const json = await oauth.processRefreshTokenResponse(as, client, response);
    const expiresIn = typeof json.expires_in === "number" ? json.expires_in : undefined;
    const next: OidcSession = {
      ...session,
      accessToken: json.access_token,
      refreshToken: json.refresh_token ?? session.refreshToken,
      expiresAtMs: expiresIn ? Date.now() + expiresIn * 1000 : undefined,
    };
    saveSession(opts, next);
    return next;
  } catch {
    // Refresh failed — leave the existing (possibly-expired) session in
    // place; the next OHDC call will see the 401 and prompt re-login.
    return session;
  }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/**
 * Best-effort base64url-decode of the JWT body. **Not** a verified parse.
 * Used only when `idTokenClaims === "unsafe-decode"`. Server-side
 * (storage / relay) re-validates the access token on every call.
 */
function decodeJwtClaims(jwt: string): Record<string, unknown> {
  const parts = jwt.split(".");
  if (parts.length < 2) throw new Error("not a JWT");
  const body = parts[1];
  const padded = body
    .padEnd(Math.ceil(body.length / 4) * 4, "=")
    .replace(/-/g, "+")
    .replace(/_/g, "/");
  const json = atob(padded);
  return JSON.parse(json) as Record<string, unknown>;
}
