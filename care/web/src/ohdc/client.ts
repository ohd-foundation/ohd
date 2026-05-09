// OHDC client wrapper backed by a single grant token.
//
// The Care v0 web app holds exactly one grant (per browser session) and views
// the patient who owns it. v0.x will extend this to N grants ("vault") with
// a "switch_patient" UX; for now, one grant = one patient.
//
// Auth surface:
//   - Token is read from `?token=ohdg_...` on first load and persisted to
//     `sessionStorage` so reloads don't lose it. Closing the tab forgets it.
//   - Subsequent reloads pick the token up from `sessionStorage` automatically.
//   - The token is sent as `Authorization: Bearer <token>` on every request
//     via a Connect-Web Interceptor.
//
// Wire:
//   - `@connectrpc/connect-web` Connect-Protocol transport (binary or JSON
//     depending on the `useBinaryFormat` option). v1 uses binary because the
//     Rust storage server speaks binary fastest.
//   - The storage URL is `VITE_STORAGE_URL` at build time, falling back to
//     `http://localhost:18443` (the demo target).

import { create } from "@bufbuild/protobuf";
import { createClient, type Client, type Interceptor } from "@connectrpc/connect";
import { createConnectTransport } from "@connectrpc/connect-web";
import {
  AuditQueryRequestSchema,
  EventFilterSchema,
  EventInputSchema,
  OhdcService,
  PutEventsRequestSchema,
  QueryEventsRequestSchema,
  UlidSchema,
  WhoAmIRequestSchema,
  type AuditEntry,
  type Event,
  type Grant,
  type WhoAmIResponse,
} from "../gen/ohdc/v0/ohdc_pb";
import {
  canonicalQueryHash,
  type CanonicalEventFilter,
} from "./canonicalQueryHash";
import {
  appendOperatorAuditEntry,
  buildAuditTemplate,
} from "./operatorAudit";

// --- Token persistence -------------------------------------------------------

const TOKEN_STORAGE_KEY = "ohd-care-grant-token";
const STORAGE_URL_KEY = "ohd-care-storage-url";

/** Parse `?token=...` out of the current URL (if any). Browser-only. */
function readTokenFromQuery(): string | null {
  if (typeof window === "undefined") return null;
  const params = new URLSearchParams(window.location.search);
  const t = params.get("token");
  if (t && t.length > 0) {
    // Strip the `?token=` from the URL so reload + bookmark don't re-leak it.
    // sessionStorage is the persistence layer from this point.
    const url = new URL(window.location.href);
    url.searchParams.delete("token");
    window.history.replaceState({}, "", url.toString());
    return t;
  }
  return null;
}

/** Resolve the active grant token via query-string-then-sessionStorage. */
export function resolveGrantToken(): string | null {
  if (typeof window === "undefined") return null;
  const fromQuery = readTokenFromQuery();
  if (fromQuery) {
    sessionStorage.setItem(TOKEN_STORAGE_KEY, fromQuery);
    return fromQuery;
  }
  return sessionStorage.getItem(TOKEN_STORAGE_KEY);
}

/** Forget the active grant — drops the bearer for subsequent calls. */
export function forgetGrantToken(): void {
  if (typeof window === "undefined") return;
  sessionStorage.removeItem(TOKEN_STORAGE_KEY);
}

/** Read the configured storage base URL. Honours `VITE_STORAGE_URL` first. */
export function resolveStorageUrl(): string {
  // Vite injects `import.meta.env.VITE_*` at build time. We also support
  // `?storage=URL` as a one-shot override (mirrors the grant share-URL form
  // documented in spec/care-auth.md).
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
  return "http://localhost:18443";
}

// --- Transport + client ------------------------------------------------------

/** Append `Authorization: Bearer <token>` to every outgoing request. */
function authInterceptor(getToken: () => string | null): Interceptor {
  return (next) => async (req) => {
    const token = getToken();
    if (token) {
      req.header.set("Authorization", `Bearer ${token}`);
    }
    // Attach the operator's OIDC subject so the storage's two-sided
    // audit can later JOIN "which clinician initiated the access" to
    // the patient-side audit row keyed by `grant_id`. Per
    // `spec/docs/design/care-auth.md` "Two-sided audit". Storage
    // ignores the header today; this is the integration point.
    const operatorSubject = readOperatorSubject();
    if (operatorSubject) {
      req.header.set("x-ohd-operator-subject", operatorSubject);
    }
    return await next(req);
  };
}

/** Best-effort read of the operator's OIDC `sub` claim. */
function readOperatorSubject(): string | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = sessionStorage.getItem("ohd-care-operator-session");
    if (!raw) return null;
    const session = JSON.parse(raw) as { oidcSubject?: string };
    return session.oidcSubject ?? null;
  } catch {
    return null;
  }
}

/**
 * Build a Connect-Web transport against the configured storage URL. Binary
 * Protobuf framing by default — matches the Rust server's gRPC + Connect-RPC
 * tests (`application/proto`).
 */
export function buildTransport(storageUrl: string, getToken: () => string | null) {
  return createConnectTransport({
    baseUrl: storageUrl,
    useBinaryFormat: true,
    interceptors: [authInterceptor(getToken)],
  });
}

/** Promise-style client over the OHDC service. */
export function buildClient(storageUrl: string, getToken: () => string | null): Client<typeof OhdcService> {
  return createClient(OhdcService, buildTransport(storageUrl, getToken));
}

// --- High-level config singleton ---------------------------------------------

interface OhdcConfig {
  storageUrl: string;
  /** Lazy getter so the token can rotate at runtime without rebuilding the client. */
  getToken: () => string | null;
}

let cachedConfig: OhdcConfig | null = null;
let cachedClient: Client<typeof OhdcService> | null = null;

/** Idempotent: returns (and lazily builds) the singleton client. */
export function getOhdcClient(): Client<typeof OhdcService> {
  if (cachedClient && cachedConfig) return cachedClient;
  const storageUrl = resolveStorageUrl();
  const config: OhdcConfig = {
    storageUrl,
    getToken: () => resolveGrantToken(),
  };
  cachedConfig = config;
  cachedClient = buildClient(config.storageUrl, config.getToken);
  return cachedClient;
}

/** Test hook: reset the cached client so a fresh token / URL is picked up. */
export function _resetOhdcClient(): void {
  cachedClient = null;
  cachedConfig = null;
}

// --- Diagnostics -------------------------------------------------------------

/** Call OhdcService.WhoAmI; returns null if there's no token or it's invalid. */
export async function whoAmI(): Promise<WhoAmIResponse | null> {
  const token = resolveGrantToken();
  if (!token) return null;
  try {
    const client = getOhdcClient();
    const resp = await client.whoAmI(create(WhoAmIRequestSchema, {}));
    return resp;
  } catch (err) {
    // Log but don't propagate — callers fall back to a "no patient connected"
    // state when this returns null.
    // eslint-disable-next-line no-console
    console.warn("OHDC WhoAmI failed", err);
    return null;
  }
}

// --- Reads -------------------------------------------------------------------

export interface QueryOpts {
  fromMs?: number;
  toMs?: number;
  eventTypes?: string[];
  limit?: number;
}

/** Stream and collect every Event that matches the filter. */
export async function queryEvents(opts: QueryOpts = {}): Promise<Event[]> {
  const client = getOhdcClient();
  const filter = create(EventFilterSchema, {
    fromMs: opts.fromMs != null ? BigInt(opts.fromMs) : undefined,
    toMs: opts.toMs != null ? BigInt(opts.toMs) : undefined,
    eventTypesIn: opts.eventTypes ?? [],
    includeSuperseded: true,
    limit: opts.limit != null ? BigInt(opts.limit) : undefined,
  });
  // Compute the operator-side audit row's `query_hash` BEFORE the call so it
  // matches storage's audit row even on the rejected / errored paths. Per
  // `care/SPEC.md` §7.3 the patient-side audit pre-existed; the operator-side
  // is what we control.
  const canonicalFilter: CanonicalEventFilter = {
    fromMs: opts.fromMs ?? null,
    toMs: opts.toMs ?? null,
    eventTypesIn: opts.eventTypes ?? [],
    includeSuperseded: true,
    limit: opts.limit ?? null,
  };
  const queryHash = await canonicalQueryHash("query_events", canonicalFilter);
  const auditTemplate = buildAuditTemplate(
    "query_events",
    "query_events",
    canonicalFilter,
    queryHash,
  );
  const req = create(QueryEventsRequestSchema, { filter });
  const out: Event[] = [];
  try {
    for await (const event of client.queryEvents(req)) {
      out.push(event);
    }
    appendOperatorAuditEntry({
      ...auditTemplate,
      result: "success",
      rowsReturned: out.length,
    });
  } catch (err) {
    appendOperatorAuditEntry({
      ...auditTemplate,
      result: "error",
      reason: (err as Error).message ?? String(err),
    });
    throw err;
  }
  return out;
}

// --- Writes ------------------------------------------------------------------

export interface PutEventOpts {
  eventType: string;
  /** Unix ms since epoch. */
  timestampMs: number;
  /**
   * Channels to emit. Each entry is `(channel_path, value)`. The shape of the
   * value drives the oneof in `ChannelValue`.
   */
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

/** Submit one event. Returns the per-row outcome. */
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

// --- ULID helpers ------------------------------------------------------------

const CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/**
 * Render a 16-byte ULID as Crockford-base32 (26 chars). Mirrors what the
 * Rust core's `ulid::to_crockford` emits, so we can compare wire ULIDs to
 * audit-log strings byte-for-byte.
 */
export function ulidToCrockford(bytes: Uint8Array | undefined): string {
  if (!bytes || bytes.length !== 16) return "";
  // 16 bytes = 128 bits. Crockford-base32 emits 26 chars covering 130 bits;
  // the leading char is masked to 3 bits of the first byte.
  let out = "";
  // First char: top 3 bits of byte 0.
  out += CROCKFORD[(bytes[0] >> 5) & 0x07];
  // Build a bit buffer and pull 5-bit groups starting at bit 3 of byte 0.
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

// --- Pending approval / rejection -------------------------------------------

import {
  ApprovePendingRequestSchema,
  ListPendingRequestSchema,
  RejectPendingRequestSchema,
  type PendingEvent,
} from "../gen/ohdc/v0/ohdc_pb";

/**
 * Convert a Crockford-base32 ULID string to the wire ``Ulid`` proto's
 * 16-byte ``bytes`` field. The patient-side audit row stores ULIDs as the
 * raw 16 bytes; storage decodes Crockford on the way in.
 */
export function crockfordToUlidBytes(s: string): Uint8Array {
  if (s.length !== 26) throw new Error(`ULID must be 26 chars, got ${s.length}`);
  // Decode 26 chars * 5 bits = 130 bits; we use 128 (the high 2 bits of the
  // first char are zero by ULID's spec).
  const lookup = new Map<string, number>();
  const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";
  for (let i = 0; i < ALPHABET.length; i++) lookup.set(ALPHABET[i], i);
  // Crockford allows I, L, O — rewrite those to 1/1/0 per the spec.
  const norm = s.toUpperCase().replace(/I|L/g, "1").replace(/O/g, "0");
  const out = new Uint8Array(16);
  let bits = 0;
  let buf = 0;
  let oi = 0;
  for (let i = 0; i < norm.length; i++) {
    const v = lookup.get(norm[i]);
    if (v === undefined) throw new Error(`invalid Crockford char: ${JSON.stringify(norm[i])}`);
    buf = (buf << 5) | v;
    bits += 5;
    while (bits >= 8) {
      bits -= 8;
      if (oi < 16) out[oi++] = (buf >> bits) & 0xff;
    }
  }
  return out;
}

/** List the operator's own pending submissions for the active patient.
 *
 * Storage scopes results to ``submitting_grant_ulid = caller's grant`` for
 * grant tokens, so this shows only the calling operator's queue, not the
 * patient's other queued submissions from Connect / sync.
 */
export async function listPending(): Promise<PendingEvent[]> {
  const client = getOhdcClient();
  const req = create(ListPendingRequestSchema, { status: "pending" });
  // Pending listing isn't a "read RPC" in storage's pending-query sense
  // (it's its own admin path), so we don't compute a query_hash. We do
  // record an audit row with kind=null per the §7.2 schema.
  const template = buildAuditTemplate("list_pending", null, null, null);
  try {
    const resp = await client.listPending(req);
    appendOperatorAuditEntry({ ...template, result: "success", rowsReturned: resp.pending.length });
    return resp.pending;
  } catch (err) {
    appendOperatorAuditEntry({
      ...template,
      result: "error",
      reason: (err as Error).message ?? String(err),
    });
    throw err;
  }
}

/** Approve one pending submission (operator-side; the *patient* normally
 * does this — Care v0.x exposes the operator-side path so trusted
 * deployments can pre-approve from the operator console).
 *
 * If ``alsoAutoApproveThisType`` is true, storage adds the event_type to
 * the grant's ``auto_approve_event_types`` allowlist for future writes —
 * the §6.1 "trust forever" path.
 */
export async function approvePending(
  pendingUlidCrockford: string,
  alsoAutoApproveThisType: boolean = false,
): Promise<{ committedAtMs: number; eventUlid?: Uint8Array }> {
  const client = getOhdcClient();
  const req = create(ApprovePendingRequestSchema, {
    pendingUlid: create(UlidSchema, { bytes: crockfordToUlidBytes(pendingUlidCrockford) }),
    alsoAutoApproveThisType,
  });
  const template = buildAuditTemplate("approve_pending", null, null, null);
  try {
    const resp = await client.approvePending(req);
    appendOperatorAuditEntry({ ...template, result: "success" });
    return {
      committedAtMs: Number(resp.committedAtMs),
      eventUlid: resp.eventUlid?.bytes,
    };
  } catch (err) {
    appendOperatorAuditEntry({
      ...template,
      result: "error",
      reason: (err as Error).message ?? String(err),
    });
    throw err;
  }
}

/** Reject one pending submission with an optional reason. */
export async function rejectPending(
  pendingUlidCrockford: string,
  reason?: string,
): Promise<{ rejectedAtMs: number }> {
  const client = getOhdcClient();
  const req = create(RejectPendingRequestSchema, {
    pendingUlid: create(UlidSchema, { bytes: crockfordToUlidBytes(pendingUlidCrockford) }),
    reason,
  });
  const template = buildAuditTemplate("reject_pending", null, null, null);
  try {
    const resp = await client.rejectPending(req);
    appendOperatorAuditEntry({ ...template, result: "success" });
    return { rejectedAtMs: Number(resp.rejectedAtMs) };
  } catch (err) {
    appendOperatorAuditEntry({
      ...template,
      result: "error",
      reason: (err as Error).message ?? String(err),
    });
    throw err;
  }
}

// --- Audit (two-sided) ------------------------------------------------------

/** Filter for `OhdcService.AuditQuery`. All fields optional — server defaults apply. */
export interface AuditQueryFilter {
  fromMs?: number;
  toMs?: number;
  /** Crockford-base32 ULID string, e.g. from a grant artifact. */
  grantUlid?: string;
  /** "self" | "grant" | "system". */
  actorType?: string;
  /** "read" | "write" | "grant_create" | … */
  action?: string;
  /** "success" | "partial" | "rejected" | "error". */
  result?: string;
  /** When true, the server keeps the stream open and pushes new rows as they land. */
  tail?: boolean;
}

/**
 * Stream patient-side audit rows. Storage scopes results: self-session sees
 * everything; a grant token only sees rows tagged with that grant's ULID.
 *
 * Per `care/SPEC.md` §7.3 the operator pairs each row with the
 * corresponding entry from `operatorAudit.ts` by re-hashing
 * `(query_kind, query_params_json)` and JOINing on the resulting hash —
 * see `pages/AuditPage.tsx`.
 */
export async function auditQuery(filter: AuditQueryFilter = {}): Promise<AuditEntry[]> {
  const client = getOhdcClient();
  const req = create(AuditQueryRequestSchema, {
    fromMs: filter.fromMs != null ? BigInt(filter.fromMs) : undefined,
    toMs: filter.toMs != null ? BigInt(filter.toMs) : undefined,
    grantUlid: filter.grantUlid
      ? create(UlidSchema, { bytes: crockfordToUlidBytes(filter.grantUlid) })
      : undefined,
    actorType: filter.actorType,
    action: filter.action,
    result: filter.result,
    tail: filter.tail ?? false,
  });
  // The audit query itself is recorded in our local audit log so the panel
  // can show the operator inspecting the audit log (transparency). We don't
  // compute a `query_hash` for it — `audit_query` isn't one of the five
  // canonical pending-query kinds.
  const template = buildAuditTemplate("audit_query", null, null, null);
  const out: AuditEntry[] = [];
  try {
    for await (const entry of client.auditQuery(req)) {
      out.push(entry);
    }
    appendOperatorAuditEntry({
      ...template,
      result: "success",
      rowsReturned: out.length,
    });
  } catch (err) {
    appendOperatorAuditEntry({
      ...template,
      result: "error",
      reason: (err as Error).message ?? String(err),
    });
    throw err;
  }
  return out;
}

// Re-export schemas the call sites need to construct things directly.
export {
  EventFilterSchema,
  EventInputSchema,
  PutEventsRequestSchema,
  QueryEventsRequestSchema,
  WhoAmIRequestSchema,
};
export type { AuditEntry, Event, Grant, PendingEvent, WhoAmIResponse };
