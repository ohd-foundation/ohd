// OHDC client wrapper for the dispatch console.
//
// The dispatch console authenticates as an **operator session** (not a
// patient grant) — the bearer token here represents the operator's
// authority cert + dispatcher identity, scoped to the cases their station
// is the authority for. The wire surface is the same OHDC Connect-RPC
// service; only the token semantics differ from care/web.
//
// Token resolution order:
//   1. `?token=...` on the URL (typed by the dispatcher at first launch).
//   2. `localStorage` — the dispatch console runs on the operator's
//      hardware so a longer-lived persistence is appropriate (vs. care/web
//      which uses sessionStorage to limit grant exposure).
//   3. `VITE_DISPATCH_TOKEN` build-time fallback — convenient for ops
//      runbooks ("paste this and refresh") but never the primary path.

import { create } from "@bufbuild/protobuf";
import { createClient, type Client, type Interceptor } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";
import {
  AuditQueryRequestSchema,
  CloseCaseRequestSchema,
  GetCaseRequestSchema,
  ListCasesRequestSchema,
  OhdcService,
  WhoAmIRequestSchema,
  type AuditEntry,
  type Case,
  type WhoAmIResponse,
} from "../gen/ohdc/v0/ohdc_pb.js";

// --- Token + URL persistence -----------------------------------------------

const TOKEN_STORAGE_KEY = "ohd-dispatch-operator-token";
const STORAGE_URL_KEY = "ohd-dispatch-storage-url";

function readTokenFromQuery(): string | null {
  if (typeof window === "undefined") return null;
  const params = new URLSearchParams(window.location.search);
  const t = params.get("token");
  if (t && t.length > 0) {
    const url = new URL(window.location.href);
    url.searchParams.delete("token");
    window.history.replaceState({}, "", url.toString());
    return t;
  }
  return null;
}

/** Resolve the active operator token (URL → localStorage → build-time env). */
export function resolveOperatorToken(): string | null {
  if (typeof window === "undefined") return null;
  const fromQuery = readTokenFromQuery();
  if (fromQuery) {
    localStorage.setItem(TOKEN_STORAGE_KEY, fromQuery);
    return fromQuery;
  }
  const fromStorage = localStorage.getItem(TOKEN_STORAGE_KEY);
  if (fromStorage) return fromStorage;
  const fromEnv = (import.meta.env?.VITE_DISPATCH_TOKEN as string | undefined) ?? "";
  return fromEnv || null;
}

/** Set or replace the operator token via the paste-token UI. */
export function setOperatorToken(token: string): void {
  if (typeof window === "undefined") return;
  if (!token) {
    localStorage.removeItem(TOKEN_STORAGE_KEY);
  } else {
    localStorage.setItem(TOKEN_STORAGE_KEY, token);
  }
  _resetOhdcClient();
}

export function forgetOperatorToken(): void {
  if (typeof window === "undefined") return;
  localStorage.removeItem(TOKEN_STORAGE_KEY);
  _resetOhdcClient();
}

/** Read the configured storage base URL. Honours `VITE_STORAGE_URL` first. */
export function resolveStorageUrl(): string {
  if (typeof window !== "undefined") {
    const params = new URLSearchParams(window.location.search);
    const fromQuery = params.get("storage");
    if (fromQuery) {
      localStorage.setItem(STORAGE_URL_KEY, fromQuery);
      const url = new URL(window.location.href);
      url.searchParams.delete("storage");
      window.history.replaceState({}, "", url.toString());
      return fromQuery;
    }
    const fromStorage = localStorage.getItem(STORAGE_URL_KEY);
    if (fromStorage) return fromStorage;
  }
  const fromEnv = (import.meta.env?.VITE_STORAGE_URL as string | undefined) ?? "";
  if (fromEnv) return fromEnv;
  return "http://localhost:18443";
}

/** Update the storage URL (paste-URL UI). Resets the cached client. */
export function setStorageUrl(url: string): void {
  if (typeof window === "undefined") return;
  if (!url) {
    localStorage.removeItem(STORAGE_URL_KEY);
  } else {
    localStorage.setItem(STORAGE_URL_KEY, url);
  }
  _resetOhdcClient();
}

// --- Transport + client ----------------------------------------------------

function authInterceptor(getToken: () => string | null): Interceptor {
  return (next) => async (req) => {
    const token = getToken();
    if (token) {
      req.header.set("Authorization", `Bearer ${token}`);
    }
    return await next(req);
  };
}

export function buildTransport(storageUrl: string, getToken: () => string | null) {
  return createConnectTransport({
    baseUrl: storageUrl,
    useBinaryFormat: true,
    interceptors: [authInterceptor(getToken)],
  });
}

export function buildClient(
  storageUrl: string,
  getToken: () => string | null,
): Client<typeof OhdcService> {
  return createClient(OhdcService, buildTransport(storageUrl, getToken));
}

let cachedClient: Client<typeof OhdcService> | null = null;

export function getOhdcClient(): Client<typeof OhdcService> {
  if (cachedClient) return cachedClient;
  cachedClient = buildClient(resolveStorageUrl(), () => resolveOperatorToken());
  return cachedClient;
}

/** Test hook / token rotation: drop the cached transport. */
export function _resetOhdcClient(): void {
  cachedClient = null;
}

// --- Diagnostics -----------------------------------------------------------

export async function whoAmI(): Promise<WhoAmIResponse | null> {
  const token = resolveOperatorToken();
  if (!token) return null;
  try {
    const client = getOhdcClient();
    const resp = await client.whoAmI(create(WhoAmIRequestSchema, {}));
    return resp;
  } catch (err) {
    // eslint-disable-next-line no-console
    console.warn("OHDC WhoAmI failed", err);
    return null;
  }
}

// --- Cases -----------------------------------------------------------------

/** List active (and optionally closed) cases the operator has access to. */
export async function listCases(includeClosed: boolean): Promise<Case[]> {
  const client = getOhdcClient();
  const req = create(ListCasesRequestSchema, { includeClosed });
  const resp = await client.listCases(req);
  return resp.cases;
}

export async function getCase(ulidBytes: Uint8Array): Promise<Case | null> {
  const client = getOhdcClient();
  try {
    return await client.getCase(
      create(GetCaseRequestSchema, { caseUlid: { bytes: ulidBytes } }),
    );
  } catch (err) {
    // eslint-disable-next-line no-console
    console.warn("OHDC GetCase failed", err);
    return null;
  }
}

/**
 * Force-close a case via OHDC `CloseCase`.
 *
 * NOTE on authority: per `../spec/emergency-trust.md`, force-close from the
 * operator side closes the *operator's* case grant. The patient retains
 * the right to keep their own copy of the events open. Storage today
 * accepts CloseCase from any holder of a case-bound write grant; the
 * dispatcher is the operator-level holder for cases under this station's
 * authority. The button is gated behind a confirmation in the UI.
 */
export async function closeCase(
  ulidBytes: Uint8Array,
  reason?: string,
): Promise<Case | null> {
  const client = getOhdcClient();
  try {
    return await client.closeCase(
      create(CloseCaseRequestSchema, {
        caseUlid: { bytes: ulidBytes },
        reason,
      }),
    );
  } catch (err) {
    // eslint-disable-next-line no-console
    console.warn("OHDC CloseCase failed", err);
    return null;
  }
}

// --- Audit -----------------------------------------------------------------

export interface AuditQueryOpts {
  fromMs?: number;
  toMs?: number;
  responder?: string;
  result?: string;
}

export async function auditQuery(opts: AuditQueryOpts = {}): Promise<AuditEntry[]> {
  const client = getOhdcClient();
  const req = create(AuditQueryRequestSchema, {
    fromMs: opts.fromMs != null ? BigInt(opts.fromMs) : undefined,
    toMs: opts.toMs != null ? BigInt(opts.toMs) : undefined,
    result: opts.result,
  });
  const out: AuditEntry[] = [];
  try {
    for await (const entry of client.auditQuery(req)) {
      out.push(entry);
    }
  } catch (err) {
    // Storage today returns NOT_IMPLEMENTED on AuditQuery — surface the
    // error to the caller so the page shows the "TBD: storage AuditQuery
    // RPC pending" placeholder rather than a silent empty list.
    throw err;
  }
  return out;
}

// Re-exports
export type { AuditEntry, Case, WhoAmIResponse };
