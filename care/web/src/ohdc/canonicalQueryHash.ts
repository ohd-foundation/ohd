// Canonical query-hash: byte-identical to storage's
// `pending_queries::enqueue` / `lookup_decision` algorithm. Any drift between
// this implementation and storage's `serde_json::to_string(filter)` breaks the
// two-sided audit JOIN per `care/SPEC.md` §7.3 (and the per-query approval
// dedup path in `storage/spec/privacy-access.md`).
//
// Algorithm, per `storage/crates/ohd-storage-core/src/pending_queries.rs`:
//
//   sha256(query_kind || 0x00 || serde_json::to_string(filter))
//   stored as the hex-encoded 32-byte digest.
//
// `serde_json::to_string` for the storage `EventFilter` struct emits, in
// **declaration order**:
//
//   from_ms, to_ms, event_types_in, event_types_not_in, include_deleted,
//   include_superseded, limit, device_id_in, source_in, event_ulids_in,
//   sensitivity_classes_in, sensitivity_classes_not_in, channel_predicates,
//   case_ulids_in
//
// Notes:
// - `Option<T>` fields with no `#[serde(skip_serializing_if = …)]` serialize
//   to `null` when None — they are NOT omitted. We mirror that by emitting
//   `null` for unset numeric fields.
// - `Vec<T>` defaults to `[]`.
// - `include_superseded` defaults to `true` (Rust `default_true`).
// - `include_deleted` defaults to `false`.
// - Compact JSON (no whitespace) matches `to_string` (vs `to_string_pretty`).
//
// To regenerate golden vectors, run the Rust helper at
// `care/cli/tests/golden_query_hash.rs` (TBD when storage exposes it as a
// `cargo run` target). For now, golden vectors live in
// `golden_query_hash.json` and are asserted by the unit tests on every side.

/**
 * Storage-aligned EventFilter shape. Mirrors the Rust struct field order; only
 * the fields that go on the wire are part of the canonical hash.
 *
 * Care call sites build this from their own typed inputs (`QueryOpts`,
 * etc.). Fields not specified default to the storage-side default for
 * that field — `null` for `Option`, `[]` for `Vec`, `false`/`true` for the
 * two booleans (matching the Rust defaults).
 */
export interface CanonicalEventFilter {
  fromMs?: number | bigint | null;
  toMs?: number | bigint | null;
  eventTypesIn?: string[];
  eventTypesNotIn?: string[];
  includeDeleted?: boolean;
  /** Defaults to `true` (matches Rust `default_true`). */
  includeSuperseded?: boolean;
  limit?: number | bigint | null;
  deviceIdIn?: string[];
  sourceIn?: string[];
  eventUlidsIn?: string[];
  sensitivityClassesIn?: string[];
  sensitivityClassesNotIn?: string[];
  channelPredicates?: CanonicalChannelPredicate[];
  caseUlidsIn?: string[];
}

/**
 * Channel-value predicate. Mirrors the Rust `ChannelPredicate` field order
 * — `channel_path`, `op`, `value`. The `value` shape mirrors `ChannelScalar`
 * from `events.rs`: an externally-tagged enum like
 * `{"Real":1.0}` / `{"Int":2}` / `{"Bool":true}` / `{"Text":"x"}` /
 * `{"EnumOrdinal":3}`.
 */
export interface CanonicalChannelPredicate {
  channelPath: string;
  /** One of `eq | neq | gt | gte | lt | lte`. */
  op: string;
  value:
    | { Real: number }
    | { Int: number | bigint }
    | { Bool: boolean }
    | { Text: string }
    | { EnumOrdinal: number };
}

/** OHDC `query_kind` strings recognized by storage's pending-query path. */
export type CanonicalQueryKind =
  | "query_events"
  | "aggregate"
  | "correlate"
  | "read_samples"
  | "read_attachment"
  | "get_event_by_ulid";

/**
 * Render the canonical JSON for a filter. Pure function; no I/O. Useful for
 * inspecting what the audit row will key on.
 */
export function canonicalFilterJson(filter: CanonicalEventFilter): string {
  const obj = {
    from_ms: numOrNull(filter.fromMs),
    to_ms: numOrNull(filter.toMs),
    event_types_in: filter.eventTypesIn ?? [],
    event_types_not_in: filter.eventTypesNotIn ?? [],
    include_deleted: filter.includeDeleted ?? false,
    include_superseded: filter.includeSuperseded ?? true,
    limit: numOrNull(filter.limit),
    device_id_in: filter.deviceIdIn ?? [],
    source_in: filter.sourceIn ?? [],
    event_ulids_in: filter.eventUlidsIn ?? [],
    sensitivity_classes_in: filter.sensitivityClassesIn ?? [],
    sensitivity_classes_not_in: filter.sensitivityClassesNotIn ?? [],
    channel_predicates: (filter.channelPredicates ?? []).map(canonicalPredicate),
    case_ulids_in: filter.caseUlidsIn ?? [],
  };
  // `JSON.stringify` with no replacer / spacer matches `serde_json::to_string`'s
  // compact form (no whitespace). Object key order is iteration order, which
  // is the order we wrote the literal above — i.e. the storage struct order.
  return JSON.stringify(obj);
}

function canonicalPredicate(p: CanonicalChannelPredicate): {
  channel_path: string;
  op: string;
  value: unknown;
} {
  return { channel_path: p.channelPath, op: p.op, value: p.value };
}

function numOrNull(v: number | bigint | null | undefined): number | null {
  if (v == null) return null;
  if (typeof v === "bigint") {
    // serde_json renders i64 as a JSON number. Care passes 64-bit timestamps
    // as `bigint`; we widen to `number`. Caller is responsible for keeping
    // values within the safe-integer range (Care's filters use ms-since-epoch
    // and event-count limits, both well within 2^53).
    if (v > Number.MAX_SAFE_INTEGER || v < Number.MIN_SAFE_INTEGER) {
      throw new RangeError(
        `canonicalFilterJson: bigint ${v} exceeds JS safe-integer range; cannot match storage's i64 JSON encoding`,
      );
    }
    return Number(v);
  }
  return v;
}

/**
 * Compute the byte-identical query hash that storage records on the patient
 * side. Returns hex of the 32-byte SHA-256 digest. Used both to dedup pending
 * approvals on the storage side and to JOIN the operator-side audit row to
 * the patient-side audit row per `care/SPEC.md` §7.3.
 *
 * Browser-only: uses the Web Crypto SubtleCrypto API. For Node tests we use
 * the same API via `globalThis.crypto.subtle` (Node ≥ 19 ships it).
 */
export async function canonicalQueryHash(
  queryKind: CanonicalQueryKind,
  filter: CanonicalEventFilter,
): Promise<string> {
  const payload = canonicalFilterJson(filter);
  const enc = new TextEncoder();
  const kindBytes = enc.encode(queryKind);
  const sep = new Uint8Array([0x00]);
  const payloadBytes = enc.encode(payload);
  const merged = new Uint8Array(kindBytes.length + 1 + payloadBytes.length);
  merged.set(kindBytes, 0);
  merged.set(sep, kindBytes.length);
  merged.set(payloadBytes, kindBytes.length + 1);
  const digest = await crypto.subtle.digest("SHA-256", merged);
  return bytesToHex(new Uint8Array(digest));
}

function bytesToHex(b: Uint8Array): string {
  let s = "";
  for (let i = 0; i < b.length; i++) {
    const hex = b[i].toString(16);
    if (hex.length === 1) s += "0";
    s += hex;
  }
  return s;
}

/**
 * Sync wrapper: same hash via a pluggable digester. Useful for tests that
 * drive a Node-style synchronous SHA-256. Returns the same hex string.
 */
export function canonicalQueryHashWith(
  queryKind: CanonicalQueryKind,
  filter: CanonicalEventFilter,
  sha256: (buf: Uint8Array) => Uint8Array,
): string {
  const payload = canonicalFilterJson(filter);
  const enc = new TextEncoder();
  const kindBytes = enc.encode(queryKind);
  const sep = new Uint8Array([0x00]);
  const payloadBytes = enc.encode(payload);
  const merged = new Uint8Array(kindBytes.length + 1 + payloadBytes.length);
  merged.set(kindBytes, 0);
  merged.set(sep, kindBytes.length);
  merged.set(payloadBytes, kindBytes.length + 1);
  return bytesToHex(sha256(merged));
}

/**
 * Compute the canonical query-hash from a raw `query_params_json` string.
 *
 * Storage's `AuditEntry` carries `query_kind` and `query_params_json` —
 * the same canonical compact JSON that `canonicalFilterJson` emits on
 * our side. To JOIN the operator-side audit row to the patient-side
 * audit row in `care/web/src/pages/AuditPage.tsx`, we re-hash storage's
 * row using this helper and look the hash up in the operator-side
 * audit log.
 *
 * Note this accepts `string` for `queryKind` because storage emits raw
 * action strings ("read", "write", …) for non-pending-query audit rows;
 * we only get a hash that's joinable to the operator side when
 * `query_kind` is one of the OHDC pending-query kinds. Callers that
 * know they're outside that set (writes, list_pending, …) skip the
 * hash and JOIN by `(grant_ulid, ts_ms ± window, action)` instead.
 */
export async function canonicalQueryHashFromRawJson(
  queryKind: string,
  paramsJson: string,
): Promise<string> {
  const enc = new TextEncoder();
  const kindBytes = enc.encode(queryKind);
  const sep = new Uint8Array([0x00]);
  const payloadBytes = enc.encode(paramsJson);
  const merged = new Uint8Array(kindBytes.length + 1 + payloadBytes.length);
  merged.set(kindBytes, 0);
  merged.set(sep, kindBytes.length);
  merged.set(payloadBytes, kindBytes.length + 1);
  const digest = await crypto.subtle.digest("SHA-256", merged);
  return bytesToHex(new Uint8Array(digest));
}
