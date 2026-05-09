// OHDC client wrapper for the Connect web SPA.
//
// The personal-side Connect app runs under a **self-session** token (`ohds_…`),
// not a grant token. The token is acquired one of three ways for v0.1:
//   1. Query string: `?token=ohds_...` on first load. Stripped from the URL
//      after copy to sessionStorage so reload + bookmark don't re-leak it.
//   2. Paste-token textbox in the Settings → Storage page. Persisted to
//      sessionStorage.
//   3. (v0.x) OAuth 2.0 Authorization Code Flow + PKCE per
//      `connect/spec/auth.md`. Not implemented; documented in STATUS.md.
//
// Wire transport: Connect-Web binary (`useBinaryFormat: true`) — matches the
// Rust storage server's preferred codec and is the form factor under e2e
// test (`storage/crates/ohd-storage-server/tests/end_to_end.rs`).
//
// Storage URL: `VITE_STORAGE_URL` at build time (default
// `http://localhost:8443` — the storage server's default `serve --listen`).
// Overridable at runtime via `?storage=<url>` (one-shot persisted to
// sessionStorage) or via the Storage settings page.

import { create } from "@bufbuild/protobuf";
import { createClient, type Client, type Interceptor } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";
import {
  EventFilterSchema,
  EventInputSchema,
  HealthRequestSchema,
  OhdcService,
  PutEventsRequestSchema,
  QueryEventsRequestSchema,
  WhoAmIRequestSchema,
  ListGrantsRequestSchema,
  CreateGrantRequestSchema,
  RevokeGrantRequestSchema,
  ListPendingRequestSchema,
  ApprovePendingRequestSchema,
  RejectPendingRequestSchema,
  ListCasesRequestSchema,
  CloseCaseRequestSchema,
  CreateCaseRequestSchema,
  type Event,
  type Grant,
  type PendingEvent,
  type Case,
  type WhoAmIResponse,
  type HealthResponse,
} from "../gen/ohdc/v0/ohdc_pb";

// --- Token persistence -----------------------------------------------------

const TOKEN_STORAGE_KEY = "ohd-connect-self-token";
const STORAGE_URL_KEY = "ohd-connect-storage-url";

/** Parse `?token=...` out of the current URL (if any). Browser-only. */
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

/** Resolve the active self-session token via query → sessionStorage. */
export function resolveSelfToken(): string | null {
  if (typeof window === "undefined") return null;
  const fromQuery = readTokenFromQuery();
  if (fromQuery) {
    sessionStorage.setItem(TOKEN_STORAGE_KEY, fromQuery);
    _resetOhdcClient(); // ensure fresh transport on token swap
    return fromQuery;
  }
  return sessionStorage.getItem(TOKEN_STORAGE_KEY);
}

/** Set the self-session token explicitly (paste-token UX, login). */
export function setSelfToken(token: string): void {
  if (typeof window === "undefined") return;
  sessionStorage.setItem(TOKEN_STORAGE_KEY, token);
  _resetOhdcClient();
}

/** Forget the active token — drops the bearer for subsequent calls. */
export function forgetSelfToken(): void {
  if (typeof window === "undefined") return;
  sessionStorage.removeItem(TOKEN_STORAGE_KEY);
  _resetOhdcClient();
}

/** Read the configured storage base URL. Honours `VITE_STORAGE_URL` first. */
export function resolveStorageUrl(): string {
  if (typeof window !== "undefined") {
    const params = new URLSearchParams(window.location.search);
    const fromQuery = params.get("storage");
    if (fromQuery) {
      sessionStorage.setItem(STORAGE_URL_KEY, fromQuery);
      const url = new URL(window.location.href);
      url.searchParams.delete("storage");
      window.history.replaceState({}, "", url.toString());
      return fromQuery;
    }
    const fromSession = sessionStorage.getItem(STORAGE_URL_KEY);
    if (fromSession) return fromSession;
  }
  const fromEnv = (import.meta.env?.VITE_STORAGE_URL as string | undefined) ?? "";
  if (fromEnv) return fromEnv;
  return "http://localhost:8443";
}

/** Override the storage URL (Settings → Storage page). */
export function setStorageUrl(url: string): void {
  if (typeof window === "undefined") return;
  sessionStorage.setItem(STORAGE_URL_KEY, url);
  _resetOhdcClient();
}

// --- Transport + client ----------------------------------------------------

/** Append `Authorization: Bearer <token>` to every outgoing request. */
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

export function buildClient(storageUrl: string, getToken: () => string | null): Client<typeof OhdcService> {
  return createClient(OhdcService, buildTransport(storageUrl, getToken));
}

// --- High-level config singleton -------------------------------------------

interface OhdcConfig {
  storageUrl: string;
  getToken: () => string | null;
}

let cachedConfig: OhdcConfig | null = null;
let cachedClient: Client<typeof OhdcService> | null = null;

export function getOhdcClient(): Client<typeof OhdcService> {
  if (cachedClient && cachedConfig) return cachedClient;
  const storageUrl = resolveStorageUrl();
  const config: OhdcConfig = {
    storageUrl,
    getToken: () => resolveSelfToken(),
  };
  cachedConfig = config;
  cachedClient = buildClient(config.storageUrl, config.getToken);
  return cachedClient;
}

/** Test hook — also called internally on token / URL rotation. */
export function _resetOhdcClient(): void {
  cachedClient = null;
  cachedConfig = null;
}

// --- Diagnostics -----------------------------------------------------------

export async function whoAmI(): Promise<WhoAmIResponse | null> {
  const token = resolveSelfToken();
  if (!token) return null;
  try {
    const client = getOhdcClient();
    return await client.whoAmI(create(WhoAmIRequestSchema, {}));
  } catch (err) {
    // eslint-disable-next-line no-console
    console.warn("OHDC WhoAmI failed", err);
    return null;
  }
}

export async function healthCheck(): Promise<HealthResponse | null> {
  try {
    const client = getOhdcClient();
    return await client.health(create(HealthRequestSchema, {}));
  } catch (err) {
    // eslint-disable-next-line no-console
    console.warn("OHDC Health failed", err);
    return null;
  }
}

// --- Reads -----------------------------------------------------------------

export interface QueryOpts {
  fromMs?: number;
  toMs?: number;
  eventTypes?: string[];
  limit?: number;
}

export async function queryEvents(opts: QueryOpts = {}): Promise<Event[]> {
  const client = getOhdcClient();
  const filter = create(EventFilterSchema, {
    fromMs: opts.fromMs != null ? BigInt(opts.fromMs) : undefined,
    toMs: opts.toMs != null ? BigInt(opts.toMs) : undefined,
    eventTypesIn: opts.eventTypes ?? [],
    includeSuperseded: true,
    limit: opts.limit != null ? BigInt(opts.limit) : undefined,
  });
  const req = create(QueryEventsRequestSchema, { filter });
  const out: Event[] = [];
  for await (const event of client.queryEvents(req)) {
    out.push(event);
  }
  return out;
}

// --- Writes ----------------------------------------------------------------

export interface PutEventOpts {
  eventType: string;
  timestampMs: number;
  channels: Array<{
    channelPath: string;
    value:
      | { kind: "real"; realValue: number }
      | { kind: "int"; intValue: number }
      | { kind: "bool"; boolValue: boolean }
      | { kind: "text"; textValue: string }
      | { kind: "enum"; enumOrdinal: number };
  }>;
  notes?: string;
  source?: string;
  sourceId?: string;
}

export type PutEventOutcome =
  | { kind: "committed"; ulid: string }
  | { kind: "pending"; ulid: string; expiresAtMs: number }
  | { kind: "error"; code: string; message: string };

export async function putEvent(input: PutEventOpts): Promise<PutEventOutcome> {
  const client = getOhdcClient();
  const eventInput = create(EventInputSchema, {
    timestampMs: BigInt(input.timestampMs),
    eventType: input.eventType,
    channels: input.channels.map((c) => {
      switch (c.value.kind) {
        case "real":
          return {
            channelPath: c.channelPath,
            value: { case: "realValue" as const, value: c.value.realValue },
          };
        case "int":
          return {
            channelPath: c.channelPath,
            value: { case: "intValue" as const, value: BigInt(c.value.intValue) },
          };
        case "bool":
          return {
            channelPath: c.channelPath,
            value: { case: "boolValue" as const, value: c.value.boolValue },
          };
        case "text":
          return {
            channelPath: c.channelPath,
            value: { case: "textValue" as const, value: c.value.textValue },
          };
        case "enum":
          return {
            channelPath: c.channelPath,
            value: { case: "enumOrdinal" as const, value: c.value.enumOrdinal },
          };
      }
    }),
    notes: input.notes,
    source: input.source,
    sourceId: input.sourceId,
  });
  const req = create(PutEventsRequestSchema, { events: [eventInput], atomic: false });
  const resp = await client.putEvents(req);
  const result = resp.results[0];
  if (!result) {
    return { kind: "error", code: "NO_RESULT", message: "PutEvents returned no results" };
  }
  switch (result.outcome.case) {
    case "committed":
      return { kind: "committed", ulid: ulidToCrockford(result.outcome.value.ulid?.bytes) };
    case "pending":
      return {
        kind: "pending",
        ulid: ulidToCrockford(result.outcome.value.ulid?.bytes),
        expiresAtMs: Number(result.outcome.value.expiresAtMs),
      };
    case "error":
      return {
        kind: "error",
        code: result.outcome.value.code,
        message: result.outcome.value.message,
      };
    default:
      return { kind: "error", code: "EMPTY_OUTCOME", message: "outcome oneof was unset" };
  }
}

// --- Grants ----------------------------------------------------------------

export interface CreateGrantOpts {
  granteeLabel: string;
  granteeKind: string; // 'user' | 'role' | 'org' | 'researcher' | 'emergency_authority'
  purpose?: string;
  defaultAction: "allow" | "deny";
  approvalMode: "always" | "auto_for_event_types" | "never_required";
  expiresAtMs?: number;
  notifyOnAccess?: boolean;
  stripNotes?: boolean;
  aggregationOnly?: boolean;
  /** Allowed read event types (mapped to GrantEventTypeRule[] effect=allow). */
  readEventTypes?: string[];
  /** Allowed write event types (mapped to GrantWriteEventTypeRule[]). */
  writeEventTypes?: string[];
  /** Sensitivity-class denials. */
  denySensitivityClasses?: string[];
  /** Auto-approve list when approval_mode=auto_for_event_types. */
  autoApproveEventTypes?: string[];
}

export interface CreateGrantResult {
  grantUlid: string;
  token: string;
  shareUrl: string;
  grant: Grant;
}

export async function createGrant(opts: CreateGrantOpts): Promise<CreateGrantResult> {
  const client = getOhdcClient();
  const req = create(CreateGrantRequestSchema, {
    granteeLabel: opts.granteeLabel,
    granteeKind: opts.granteeKind,
    purpose: opts.purpose,
    defaultAction: opts.defaultAction,
    approvalMode: opts.approvalMode,
    aggregationOnly: !!opts.aggregationOnly,
    stripNotes: !!opts.stripNotes,
    requireApprovalPerQuery: false,
    expiresAtMs: opts.expiresAtMs != null ? BigInt(opts.expiresAtMs) : undefined,
    notifyOnAccess: !!opts.notifyOnAccess,
    eventTypeRules: (opts.readEventTypes ?? []).map((et) => ({ eventType: et, effect: "allow" })),
    writeEventTypeRules: (opts.writeEventTypes ?? []).map((et) => ({ eventType: et, effect: "allow" })),
    sensitivityRules: (opts.denySensitivityClasses ?? []).map((sc) => ({ sensitivityClass: sc, effect: "deny" })),
    autoApproveEventTypes: opts.autoApproveEventTypes ?? [],
  });
  const resp = await client.createGrant(req);
  return {
    grantUlid: ulidToCrockford(resp.grant?.ulid?.bytes),
    token: resp.token,
    shareUrl: resp.shareUrl,
    grant: resp.grant!,
  };
}

export async function listGrants(includeRevoked = false): Promise<Grant[]> {
  const client = getOhdcClient();
  const req = create(ListGrantsRequestSchema, { includeRevoked, includeExpired: false });
  const resp = await client.listGrants(req);
  return resp.grants;
}

export async function revokeGrant(grantUlidStr: string, reason?: string): Promise<number> {
  const client = getOhdcClient();
  const req = create(RevokeGrantRequestSchema, {
    grantUlid: { bytes: crockfordToBytes(grantUlidStr) },
    reason,
  });
  const resp = await client.revokeGrant(req);
  return Number(resp.revokedAtMs);
}

// --- Pending ---------------------------------------------------------------

export async function listPending(status: "pending" | "approved" | "rejected" | "expired" = "pending"): Promise<PendingEvent[]> {
  const client = getOhdcClient();
  const req = create(ListPendingRequestSchema, { status });
  const resp = await client.listPending(req);
  return resp.pending;
}

export async function approvePending(pendingUlidStr: string, alsoTrustType = false): Promise<string> {
  const client = getOhdcClient();
  const req = create(ApprovePendingRequestSchema, {
    pendingUlid: { bytes: crockfordToBytes(pendingUlidStr) },
    alsoAutoApproveThisType: alsoTrustType,
  });
  const resp = await client.approvePending(req);
  return ulidToCrockford(resp.eventUlid?.bytes);
}

export async function rejectPending(pendingUlidStr: string, reason?: string): Promise<number> {
  const client = getOhdcClient();
  const req = create(RejectPendingRequestSchema, {
    pendingUlid: { bytes: crockfordToBytes(pendingUlidStr) },
    reason,
  });
  const resp = await client.rejectPending(req);
  return Number(resp.rejectedAtMs);
}

// --- Pending read queries (require_approval_per_query) --------------------
//
// Storage core landed `pending_queries` table + `list/approve/reject_pending_query`
// helpers (see `storage/STATUS.md` "What landed" → `require_approval_per_query`,
// migration `005_pending_queries.sql`). The wire-level RPCs
// (`OhdcService.{List,Approve,Reject}PendingQuery`) are NOT yet exposed in
// `storage/proto/ohdc/v0/ohdc.proto` — they're flagged for the v1.x sweep.
//
// To unblock the connect-web UX (this is the user-facing surface that makes
// the per-query approval flag useful), v0.x falls back to an in-process mock
// store. The shape mirrors what the proto messages WILL be:
//
//   PendingQuery { ulid, grant_ulid, grant_label, query_kind, query_summary,
//                  requested_at_ms, expires_at_ms }
//   ListPendingQueriesResponse { repeated PendingQuery pending }
//   ApprovePendingQueryRequest  { ulid }
//   RejectPendingQueryRequest   { ulid, reason? }
//
// When the proto lands, swap the bodies of `listPendingQueries`,
// `approvePendingQuery`, and `rejectPendingQuery` to call
// `client.listPendingQueries(...)` etc. — the call sites (`store.ts` selector
// + `PendingQueriesPage`) won't need to change.

export interface PendingQuerySummary {
  /** Channels / event-types the grantee asked to read. */
  eventTypes: string[];
  /** From-ms of the asked window (null = unbounded). */
  fromMs: number | null;
  /** To-ms of the asked window (null = "now"). */
  toMs: number | null;
  /** Optional human-readable hint ("Wants to read: glucose, last 7 days"). */
  hint?: string;
}

export interface PendingQuery {
  /** Crockford-base32 ULID of the pending_queries row. */
  queryUlid: string;
  /** Crockford ULID of the issuing grant. */
  grantUlid: string;
  /** Human-readable grant label captured at request time. */
  grantLabel: string;
  /** "query_events" | "get_event_by_ulid" | "aggregate" | … */
  queryKind: string;
  /** Structured summary for the UI; the storage core builds this from the request. */
  summary: PendingQuerySummary;
  /** Unix-ms when the grantee made the call. */
  requestedAtMs: number;
  /** Unix-ms when the row auto-expires (rejected with `APPROVAL_TIMEOUT`). */
  expiresAtMs: number;
}

interface PendingQueryMockState {
  rows: PendingQuery[];
}

const MOCK: PendingQueryMockState = { rows: [] };

/**
 * Seed the in-memory mock with synthetic rows. Called by the bootstrap path
 * exactly once when `import.meta.env.VITE_PENDING_QUERIES_MOCK !== "off"`,
 * so reviewers can see a populated UI without a running storage server.
 *
 * Tests may also call this directly via the `_seed*` named exports.
 */
export function _seedPendingQueriesMock(rows: PendingQuery[]): void {
  MOCK.rows = [...rows];
}

/** Tests only — clear the mock store. */
export function _clearPendingQueriesMock(): void {
  MOCK.rows = [];
}

/** Tests only — read the current mock rows. */
export function _readPendingQueriesMock(): PendingQuery[] {
  return [...MOCK.rows];
}

/** True when the proto-generated `OhdcService` exposes the `listPendingQueries` RPC. */
function pendingQueriesWireExposed(): boolean {
  // Probe the generated client for the method. Today this is `false`; when
  // storage's v1.x proto sweep adds the RPCs and `pnpm gen` runs, this flips
  // automatically and the wire path takes over.
  const client = getOhdcClient() as unknown as Record<string, unknown>;
  return typeof client.listPendingQueries === "function";
}

/**
 * Shape of a `PendingQuery` row as returned by the (future) wire RPC, after
 * proto deserialization. Matches the fields documented in the spec block above
 * (`PendingQuery { ulid, grant_ulid, grant_label, query_kind, query_summary,
 * requested_at_ms, expires_at_ms }`). All ULIDs arrive as bytes-bearing
 * messages (`{ bytes: Uint8Array }`) and timestamps may be `bigint` or
 * `number` depending on protoc-gen-es defaults — we coerce with `Number()`.
 */
interface PendingQueryWire {
  ulid?: { bytes: Uint8Array };
  grantUlid?: { bytes: Uint8Array };
  grantLabel: string;
  queryKind: string;
  summary?: {
    eventTypes?: string[];
    fromMs?: number | bigint | null;
    toMs?: number | bigint | null;
    hint?: string;
  };
  requestedAtMs: number | bigint;
  expiresAtMs: number | bigint;
}

export async function listPendingQueries(): Promise<PendingQuery[]> {
  if (pendingQueriesWireExposed()) {
    // Future wire path. Kept as a stub so swapping is a one-liner.
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const client = getOhdcClient() as any;
    const resp = (await client.listPendingQueries({})) as { pending?: PendingQueryWire[] };
    return (resp.pending ?? []).map((p: PendingQueryWire) => ({
      queryUlid: ulidToCrockford(p.ulid?.bytes),
      grantUlid: ulidToCrockford(p.grantUlid?.bytes),
      grantLabel: p.grantLabel,
      queryKind: p.queryKind,
      summary: {
        eventTypes: p.summary?.eventTypes ?? [],
        fromMs: p.summary?.fromMs != null ? Number(p.summary.fromMs) : null,
        toMs: p.summary?.toMs != null ? Number(p.summary.toMs) : null,
        hint: p.summary?.hint,
      },
      requestedAtMs: Number(p.requestedAtMs),
      expiresAtMs: Number(p.expiresAtMs),
    }));
  }
  // v0.x fallback: in-memory mock.
  return [...MOCK.rows];
}

export async function approvePendingQuery(queryUlid: string): Promise<void> {
  if (pendingQueriesWireExposed()) {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const client = getOhdcClient() as any;
    await client.approvePendingQuery({ ulid: { bytes: crockfordToBytes(queryUlid) } });
    return;
  }
  MOCK.rows = MOCK.rows.filter((r) => r.queryUlid !== queryUlid);
}

export async function rejectPendingQuery(queryUlid: string, _reason?: string): Promise<void> {
  if (pendingQueriesWireExposed()) {
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const client = getOhdcClient() as any;
    await client.rejectPendingQuery({
      ulid: { bytes: crockfordToBytes(queryUlid) },
      reason: _reason,
    });
    return;
  }
  MOCK.rows = MOCK.rows.filter((r) => r.queryUlid !== queryUlid);
}

/**
 * Whether the running build is using the in-memory mock for pending queries.
 * The UI surfaces a banner when this is true so reviewers know.
 */
export function pendingQueriesIsMock(): boolean {
  return !pendingQueriesWireExposed();
}

// --- Cases -----------------------------------------------------------------

export async function listCases(includeClosed = true): Promise<Case[]> {
  const client = getOhdcClient();
  const req = create(ListCasesRequestSchema, { includeClosed });
  const resp = await client.listCases(req);
  return resp.cases;
}

export async function closeCase(caseUlidStr: string, reason?: string): Promise<Case> {
  const client = getOhdcClient();
  const req = create(CloseCaseRequestSchema, {
    caseUlid: { bytes: crockfordToBytes(caseUlidStr) },
    reason,
  });
  return await client.closeCase(req);
}

export async function createCase(caseType: string, label?: string): Promise<Case> {
  const client = getOhdcClient();
  const req = create(CreateCaseRequestSchema, {
    caseType,
    caseLabel: label,
  });
  return await client.createCase(req);
}

// --- ULID helpers ----------------------------------------------------------

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/** Render 16 ULID bytes as the canonical 26-char Crockford string. */
export function ulidToCrockford(bytes: Uint8Array | undefined): string {
  if (!bytes || bytes.length !== 16) return "";
  let out = "";
  out += CROCKFORD[(bytes[0] >> 5) & 0x07];
  let buf = bytes[0] & 0x1f;
  let bits = 5;
  for (let i = 1; i < 16; i++) {
    buf = (buf << 8) | bytes[i];
    bits += 8;
    while (bits >= 5) {
      bits -= 5;
      out += CROCKFORD[(buf >> bits) & 0x1f];
    }
  }
  if (bits > 0) {
    out += CROCKFORD[(buf << (5 - bits)) & 0x1f];
  }
  return out;
}

/** Decode a 26-char Crockford-base32 ULID into 16 bytes. */
export function crockfordToBytes(s: string): Uint8Array {
  if (!s || s.length !== 26) {
    throw new Error(`crockfordToBytes: expected 26 chars, got ${s.length}`);
  }
  const out = new Uint8Array(16);
  // First char encodes top 3 bits of byte 0.
  const first = decodeChar(s.charCodeAt(0));
  if (first > 7) throw new Error("crockfordToBytes: first char > 7 (overflow)");
  let buf = first;
  let bits = 3;
  let outIdx = 0;
  for (let i = 1; i < 26; i++) {
    buf = (buf << 5) | decodeChar(s.charCodeAt(i));
    bits += 5;
    if (bits >= 8) {
      bits -= 8;
      out[outIdx++] = (buf >> bits) & 0xff;
    }
  }
  return out;
}

function decodeChar(code: number): number {
  // '0'-'9'
  if (code >= 48 && code <= 57) return code - 48;
  // 'A'-'Z' minus I, L, O, U
  let c = code;
  if (c >= 97 && c <= 122) c -= 32; // upper
  if (c >= 65 && c <= 90) {
    const idx = CROCKFORD.indexOf(String.fromCharCode(c));
    if (idx >= 0) return idx;
  }
  throw new Error(`crockford: invalid char code ${code}`);
}

// Re-export schemas + types for convenience.
export {
  EventFilterSchema,
  EventInputSchema,
  PutEventsRequestSchema,
  QueryEventsRequestSchema,
  WhoAmIRequestSchema,
  HealthRequestSchema,
};
export type { Event, Grant, PendingEvent, Case, WhoAmIResponse, HealthResponse };
