# Design: OHDC v0 Protocol

> The wire-level contract for OHDC. Operations, message shapes, error model, pagination, idempotency, streaming, filter language, and the canonical `.proto` definitions.
>
> Pairs with [`../components/connect.md`](../components/connect.md) (which fixes the protocol shape — Connect-RPC + Protobuf — and lists operations at a glance), [`storage-format.md`](storage-format.md) (the on-disk representation these messages serialize to/from), [`auth.md`](auth.md) (token kinds + OAuth flow), [`care-auth.md`](care-auth.md) (operator-side concerns), [`encryption.md`](encryption.md), and [`notifications.md`](notifications.md).
>
> The actual `.proto` files live in the `openhealth-data/ohd-protocol` repo; the canonical text below is what those files contain. This doc is the human-readable annotation layer.

## How to read this doc

1. **Common types** first — `Ulid`, `Event`, `ChannelValue`, `SampleBlock`, etc. Every operation refers back to these.
2. **Cross-cutting concerns** next — error model, pagination, idempotency, filter language, streaming patterns. These rules apply uniformly to every RPC unless noted.
3. **Service definitions** — Connect-RPC services (`OhdcService`, `AuthService`, `SyncService`, `RelayService`) and the HTTP-only OAuth/discovery endpoints. Each operation: signature, request, response, semantics, error codes, audit behavior.
4. **Versioning** at the end.

This doc *is* the spec. If a behavior isn't documented here, it isn't part of OHDC v0; vendors implementing OHDC must not invent extensions in the `ohdc.v0.*` namespace. Custom extensions belong in vendor namespaces (e.g. `com.acme.ohdc_ext.v0.*`) and don't claim conformance.

---

## Wire shape recap

From [`../components/connect.md`](../components/connect.md):

- **Connect-RPC over HTTP/3** (HTTP/2 fallback) defined by Protobuf schemas.
- Connect's **body envelope** is the chosen streaming/error envelope. Do not
  rely on gRPC trailers for semantics; HTTP/3 trailer interop is uneven.
- Wire encoding negotiable per request: `application/proto` (binary, default) or `application/json` (Protobuf-JSON canonical encoding).
- Path prefix: `/ohdc.v0.OhdcService/<Method>`, `/ohdc.v0.AuthService/<Method>`, `/ohdc.v0.SyncService/<Method>`, `/ohdc.v0.RelayService/<Method>`.
- TLS 1.3 required, terminated by Caddy on the operator side, end-to-end through OHD Relay.
- Auth via `Authorization: Bearer <token>` header, where token is one of `ohds_…` (self-session), `ohdg_…` (grant), or `ohdd_…` (device). See [`auth.md`](auth.md) for token shapes.
- gRPC-compatible: a Connect-RPC server accepts gRPC clients, vice versa.
- HTTP/3 is served in-binary via `quinn` + `h3` on a separate UDP port with
  ALPN dispatch. Relay raw QUIC tunnel mode uses ALPN `ohd-tnl1`.

---

## Common types

```protobuf
syntax = "proto3";

package ohdc.v0;

import "google/protobuf/duration.proto";
import "google/protobuf/timestamp.proto";
import "google/protobuf/struct.proto";

// 128-bit ULID. Wire form is the canonical 16-byte big-endian binary.
// Implementations may render it as the 26-char Crockford-base32 string for human display.
message Ulid {
  bytes bytes = 1;  // exactly 16 bytes; servers reject other lengths with INVALID_ARGUMENT
}

// A typed scalar value at a leaf channel of an event's channel tree.
// The `oneof` mirrors the storage's value_type column in `event_channels`.
message ChannelValue {
  // Dot-separated channel path within the event's type, e.g. "nutrition.fat.saturated".
  // Resolves to a (event_type_id, channel_id) pair via the registry.
  string channel_path = 1;

  oneof value {
    double real_value = 2;
    int64 int_value = 3;
    bool bool_value = 4;
    string text_value = 5;
    // Ordinal index into the channel's enum_values list (per registry, append-only).
    int32 enum_ordinal = 6;
  }
}

// Reference to a sample block stored on the event. The block payload itself is
// not inlined in Event responses; clients fetch via `OhdcService.ReadSamples`.
message SampleBlockRef {
  string channel_path = 1;
  int64 t0_ms = 2;            // absolute start (Unix ms, signed)
  int64 t1_ms = 3;            // absolute end
  int32 sample_count = 4;
  // Codec ID per storage-format.md "Sample blocks":
  //   1 = delta-zigzag-varint timestamps + float32, zstd
  //   2 = delta-zigzag-varint timestamps + int16 quantized + scale, zstd
  int32 encoding = 5;
}

// Decoded sample row, returned by ReadSamples.
message Sample {
  int64 t_ms = 1;
  double value = 2;
}

// Reference to a sidecar attachment. Payload fetched via `OhdcService.ReadAttachment`.
message AttachmentRef {
  Ulid ulid = 1;
  bytes sha256 = 2;            // 32 bytes; addresses the sidecar blob
  int64 byte_size = 3;
  string mime_type = 4;        // e.g. "application/dicom", "image/png"
  string filename = 5;
}

message SourceSignature {
  string sig_alg = 1;       // 'ed25519' | 'rs256' | 'es256'
  string signer_kid = 2;
  bytes signature = 3;
}

message SignerInfo {
  string signer_kid = 1;
  string signer_label = 2;
  string sig_alg = 3;
  bool revoked = 4;
}

// Free-form string-keyed metadata. Used sparingly — only for fields that don't
// fit into typed channels (e.g. metadata.source, metadata.source_id mirrors of
// storage columns; vendor-namespaced flags).
message Metadata {
  map<string, string> entries = 1;
}

// One health event. Mirrors the on-disk event row plus its joined channels and
// sample-block / attachment refs. Returned by reads and accepted by writes.
message Event {
  // Identity
  Ulid ulid = 1;                // wire identity
  int64 timestamp_ms = 2;       // signed Unix ms; negative = pre-1970
  optional int64 duration_ms = 3;
  optional int32 tz_offset_minutes = 4;
  optional string tz_name = 5;  // IANA, e.g. "Europe/Prague"

  // Type and content
  string event_type = 6;        // e.g. "std.glucose", "com.acme.skin_lesion"
  repeated ChannelValue channels = 7;
  repeated SampleBlockRef sample_blocks = 8;
  repeated AttachmentRef attachments = 9;

  // Provenance and freeform
  optional string device_id = 10;     // logical device identifier (not the storage rowid)
  optional string app_name = 11;
  optional string app_version = 12;
  optional string source = 13;        // e.g. "health_connect:com.x.y", "manual:android"
  optional string source_id = 14;     // idempotency key from upstream
  optional string notes = 15;

  // Lifecycle
  optional Ulid superseded_by = 16;
  optional int64 deleted_at_ms = 17;

  // Reserved for sparse extension fields. Use vendor namespaces in keys
  // (e.g. "com.acme.signed_by"). Never used to carry typed event content.
  optional Metadata metadata = 18;

  // Populated when EventInput.source_signature verified at insert time.
  optional SignerInfo signed_by = 19;
}

// Sparse representation used for writes. Same shape as Event minus the
// server-assigned identity (ulid, deleted_at_ms) and minus the lifecycle
// fields the writer can't set. Included separately so the proto codegen
// makes "what a client must provide" explicit.
message EventInput {
  int64 timestamp_ms = 1;
  optional int64 duration_ms = 2;
  optional int32 tz_offset_minutes = 3;
  optional string tz_name = 4;

  string event_type = 5;
  repeated ChannelValue channels = 6;
  repeated SampleBlockInput sample_blocks = 7;
  // Attachments are uploaded via AttachBlob and then referenced by ULID
  // when the event is created (see AttachBlob below).
  repeated Ulid attachment_ulids = 8;

  optional string device_id = 9;
  optional string app_name = 10;
  optional string app_version = 11;
  optional string source = 12;
  optional string source_id = 13;
  optional string notes = 14;

  optional Ulid superseded_by = 15;
  optional Metadata metadata = 16;

  optional SourceSignature source_signature = 17;
}

// Sample block written as part of EventInput. The encoded payload is the
// compressed bytes per storage-format.md "Sample blocks" Encoding 1 or 2.
message SampleBlockInput {
  string channel_path = 1;
  int64 t0_ms = 2;
  int64 t1_ms = 3;
  int32 sample_count = 4;
  int32 encoding = 5;
  bytes data = 6;               // compressed
}
```

### Notes on common types

- **`event_type` is a string, not an integer.** The wire identity is the namespaced name (`std.glucose`, `com.acme.foo`). The storage looks it up against `event_types` to get the internal id. This lets clients build messages without consulting the file's per-instance ids.
- **`channel_path` is a string** for the same reason.
- **Timestamps are signed `int64` ms.** Negative values represent pre-1970 events. Clients must handle the full `int64` range.
- **`Ulid` is binary on the wire.** Clients format as Crockford-base32 for human display only.
- **Sample blocks: writes carry the compressed bytes inline; reads return refs (the consumer streams `ReadSamples` to get decoded `Sample` rows).** Avoids loading dozens of MB into a single response message.

---

## Error model

OHDC errors are returned as **standard HTTP status codes** with a structured body conforming to `google.rpc.Status` (the same shape Connect-RPC + gRPC use natively). Detail messages carry an `ErrorInfo` extension.

```protobuf
message ErrorInfo {
  string code = 1;              // e.g. "INVALID_UNIT", "OUT_OF_SCOPE"
  string message = 2;           // human-readable, English; UI translates
  map<string, string> metadata = 3;  // structured detail (e.g. { "channel_path": "glucose.value" })
}
```

### HTTP status mapping

| HTTP | Connect/gRPC status | When |
|---|---|---|
| 200 | OK | Success (including empty results). |
| 400 | INVALID_ARGUMENT | Malformed request — bad ULID length, missing required field, unparseable filter. |
| 401 | UNAUTHENTICATED | Missing / invalid / revoked token. |
| 403 | PERMISSION_DENIED | Token authenticated but operation out of scope. |
| 404 | NOT_FOUND | Resource doesn't exist OR is out of grant scope (deliberately ambiguous to avoid scope-leak via probe). |
| 409 | ALREADY_EXISTS | Idempotency conflict — `(source, source_id)` already used with different content; conflicting grant updates. |
| 410 | FAILED_PRECONDITION | Resource exists but state forbids the op (revoked grant, expired token, closed case, deleted event). |
| 412 | FAILED_PRECONDITION | If-Match / If-Unmodified-Since style conflict. |
| 422 | INVALID_ARGUMENT | Validation error against the registry — see `INVALID_UNIT` / `WRONG_VALUE_TYPE` etc. |
| 429 | RESOURCE_EXHAUSTED | Rate limit. `Retry-After` header set. |
| 499 | CANCELLED | Client cancelled (only for streaming RPCs). |
| 500 | INTERNAL | Server bug. Logged with a correlation id; UI shows the id for support. |
| 503 | UNAVAILABLE | Storage unreachable, e.g. relay tunnel down. Retry. |
| 504 | DEADLINE_EXCEEDED | Server-side timeout. Retry. |

### Error code catalog

The `ErrorInfo.code` enumerates structured reasons. Vendors must use these codes (not invented strings) so client-side dispatch is reliable.

#### Authentication / authorization
- `UNAUTHENTICATED` — bearer token missing or unrecognized.
- `TOKEN_EXPIRED` — token recognized but past expiry.
- `TOKEN_REVOKED` — token recognized but revoked (different from expired so clients can decide whether to refresh vs. re-auth).
- `WRONG_TOKEN_KIND` — operation requires a different token kind (e.g. self-session-only operation called with a grant).
- `OUT_OF_SCOPE` — token is valid but doesn't grant the requested operation or filter.
- `APPROVAL_TIMEOUT` — for grants with `require_approval_per_query=1`, the user didn't approve in time.

#### Validation (registry / structural)
- `UNKNOWN_TYPE` — `event_type` not in registry.
- `UNKNOWN_CHANNEL` — `channel_path` not in registry for the given type.
- `WRONG_VALUE_TYPE` — channel value oneof mismatches the channel's declared `value_type`.
- `INVALID_UNIT` — submission specified a non-canonical unit.
- `INVALID_ENUM` — enum ordinal out of range.
- `MISSING_REQUIRED_CHANNEL` — required channel absent and its parent group present.
- `INVALID_ULID` — ULID byte length or format wrong.
- `INVALID_TIMESTAMP` — timestamp outside acceptable range or wire-decode failure.
- `INVALID_FILTER` — filter expression unparseable or references unknown fields.

#### Lifecycle
- `EVENT_DELETED` — referenced event has `deleted_at_ms` set.
- `GRANT_REVOKED` — grant exists but `revoked_at_ms` set.
- `GRANT_EXPIRED` — `expires_at_ms` past.
- `CASE_CLOSED` — case-bound grant whose case has closed.
- `CASE_NOT_FOUND` — case_id referenced doesn't exist or out of scope.
- `IDEMPOTENCY_CONFLICT` — `(source, source_id)` reused with different content.

#### Resource limits
- `RATE_LIMITED` — per-grant or per-user rate limit hit. `Retry-After` advises.
- `PAYLOAD_TOO_LARGE` — event batch, attachment, or sample block exceeds limits (see "Limits" below).
- `STORAGE_FULL` — destination has no space (cache mode physical full).

#### Format / version
- `UNSUPPORTED_PROTOCOL_VERSION` — client requested a version the storage doesn't support.
- `UNSUPPORTED_ENCODING` — sample-block encoding ID unknown to this implementation.

### Audit on errors

Every rejected RPC writes an audit row with `result='rejected'` (or `'error'` for `INTERNAL` / `UNAVAILABLE` etc.). The user sees rejected attempts on their grants in their audit view, including the error code.

---

## Pagination

Cursor-based for all list operations. No offsets; cursors are opaque server-side strings.

```protobuf
message PageRequest {
  // Maximum number of items to return. Server may return fewer.
  // Default 100; max 1000. Larger requests get RESOURCE_EXHAUSTED.
  int32 limit = 1;

  // Cursor from the previous PageResponse.next_cursor. Empty = first page.
  string cursor = 2;
}

message PageResponse {
  // Server's next cursor; empty if no more pages.
  string next_cursor = 1;
}
```

Cursors encode `(last_seen_id, snapshot_token)` so iteration is stable even if writes happen during iteration. Cursors never expire within a session but may expire after 24 hours of inactivity → `INVALID_ARGUMENT` with hint to restart.

For streaming RPCs (`QueryEvents`, `AuditQuery`), there's no cursor — the server streams all matching results until the client closes or the server completes. Use streaming for large iterations; use the paginated unary form when you need positional resumability.

---

## Idempotency

Two layers:

### Event-write idempotency

Every event written carries a `(source, source_id)` pair. The storage enforces uniqueness on this pair via `idx_events_dedup`. Retries from a flaky network, change-token replays, batch reprocessing — none produce duplicates. Re-submitting the same `(source, source_id)` with **identical content** is a no-op success (returns the existing event's ULID). Re-submitting with **different content** is `IDEMPOTENCY_CONFLICT`.

`source_id` is required when `source` is present. Sources without `source_id` get no idempotency protection.

### Request-level idempotency (optional)

Clients can include a request-level idempotency key for any RPC via the `Idempotency-Key` HTTP header (32-character base64url, single-use within a 24h window). The server caches the response keyed by `(token_id, idempotency_key)` and returns the cached result on retry — including for non-event operations like `CreateGrant`. Useful for ensuring grant-creation operations don't double-fire on network retries.

If absent, every request is processed fresh. Most RPCs are naturally idempotent (reads are idempotent by definition; the `(source, source_id)` rule covers writes); request-level keys are for the rare other cases.

---

## Filter language

Used by `QueryEvents`, `Aggregate`, `Correlate`, `AuditQuery`. Structured (not stringly-typed) so it can be validated at message-decode time.

```protobuf
message EventFilter {
  // Time bounds. At least one of (from_ms, to_ms) is recommended.
  optional int64 from_ms = 1;
  optional int64 to_ms = 2;

  // Inclusion/exclusion. Empty = all matching the time bounds (subject to grant scope).
  repeated string event_types_in = 3;     // e.g. ["std.glucose", "std.meal"]
  repeated string event_types_not_in = 4;

  // Channel-level filtering — events that have these channels in any value range.
  repeated ChannelPredicate channels = 5;

  // Source / device filtering.
  repeated string source_in = 6;
  // Device IDs (logical device identifiers). When used as part of a case_filters
  // entry, this is the typical way a "case device" (a sensor in an emergency,
  // a hospital ECG bound to a stay) feeds events into a case without the
  // device having to know about cases.
  repeated string device_id_in = 7;

  // Sensitivity-class filtering.
  repeated string sensitivity_classes_in = 8;

  // Explicit ULID list. The events whose ULIDs are in this list match.
  // Used for case_filters entries that do "patient-curated explicit linking"
  // ("these specific events are evidence for this case").
  // Bounded — implementations enforce a reasonable cap (default 1000) per filter.
  repeated Ulid event_ulids_in = 9;

  // Lifecycle.
  bool include_deleted = 10;              // default false
  bool include_superseded = 11;           // default true (return both original and correction)

  // Sort and limit. For paginated forms use PageRequest; for streaming this gates total stream size.
  optional int64 limit = 12;
  optional Sort sort = 13;
}

message ChannelPredicate {
  string channel_path = 1;

  oneof predicate {
    Range real_range = 2;
    Range int_range = 3;
    bool exists = 4;                      // true = channel must be present; false = must be absent
    EnumIn enum_in = 5;
    string text_contains = 6;             // case-insensitive substring; for notes / labels
  }
}

message Range {
  optional double min = 1;
  optional double max = 2;
  bool min_inclusive = 3;
  bool max_inclusive = 4;
}

message EnumIn {
  repeated int32 ordinals = 1;
}

enum Sort {
  TIME_DESC = 0;                          // default — most recent first
  TIME_ASC = 1;
  ULID_DESC = 2;                          // by wire ulid (matches time within an era)
  ULID_ASC = 3;
}
```

The filter is **explicit and bounded**. There's no Turing-complete query language; this is intentional. Servers can compile a filter to indexed SQL trivially; query planners stay simple; permission resolution stays per-row predictable.

---

## Streaming patterns

Connect-RPC supports unary, server-streaming, client-streaming, and bidi. OHDC v0 uses:

| RPC | Pattern | Why |
|---|---|---|
| `OhdcService.QueryEvents` | server-streaming | Result set can be large; client backpressure on the stream. |
| `OhdcService.ReadSamples` | server-streaming | A single sample block can be ~1k samples; multiple blocks per request. |
| `OhdcService.AttachBlob` | client-streaming | Large attachments uploaded chunked. |
| `OhdcService.ReadAttachment` | server-streaming | Same on the read side. |
| `OhdcService.AuditQuery` | server-streaming + tail mode | Tail mode keeps the stream open and pushes new audit rows as they appear. |
| `OhdcService.Export` | server-streaming | Full export is large; chunked NDJSON-like framing. |
| `OhdcService.Import` | client-streaming | Same on the import side. |
| All others | unary | Small request/response. |

### Streaming framing

For server-streaming reads (`QueryEvents`, `AuditQuery`), the stream's element is the message type itself (one `Event` per stream message, one `AuditEntry` per stream message). Backpressure is the Connect-RPC stream's natural HTTP/2 / HTTP/3 flow control.

For chunked transfer (`AttachBlob`, `ReadAttachment`, `Export`, `Import`), the stream's element is a chunk message:

```protobuf
message AttachBlobChunk {
  oneof content {
    AttachBlobInit init = 1;             // first message: metadata
    bytes data = 2;                      // subsequent messages: payload bytes
    AttachBlobFinish finish = 3;         // last message: assertion the stream is complete
  }
}

message AttachBlobInit {
  string mime_type = 1;
  string filename = 2;
  // Optional ULID the client wants the resulting attachment to use. If not set, server mints one.
  // If the same ULID has already been used (idempotency), the server returns the existing AttachmentRef
  // when the bytes match (sha256 check), or ALREADY_EXISTS when they don't.
  optional Ulid ulid = 3;
  // Total expected byte size, for early rejection on size limit.
  int64 expected_byte_size = 4;
}

message AttachBlobFinish {
  bytes expected_sha256 = 1;             // 32 bytes; server verifies on receipt
}
```

Symmetric shape for `ReadAttachment` (server emits init, then chunks, then finish), `Export` (chunks of NDJSON-equivalent frames), and `Import`.

### Cancellation and resumability

- A client closing the stream prematurely → server cancels the operation (writes a `result='cancelled'` audit row).
- A server-side error mid-stream → emits a final error frame with `Status`; client surfaces.
- For `Export` / `Import`, resumability uses `(export_token, byte_offset)` exchanged on init. If the stream breaks, the client retries with the offset; the server seeks and resumes. Export tokens valid for 24h.

---

## Limits

| Limit | Value | Why |
|---|---|---|
| Single event size (encoded EventInput) | 256 KB | Catches accidentally-huge `notes` or wrong-table-of-data submissions. |
| Single batch (`PutEvents.events`) | 1000 events | Throughput vs. transaction size. |
| Single sample block (`SampleBlockInput.data` after decompress) | 1 MB | One block ≈ 15 min × dense channel; well under 1 MB. |
| Single attachment | 100 MB default; configurable per deployment up to 1 GB | Chunked upload absorbs the size; deployment can raise. |
| Single QueryEvents stream | 100k events default | Use Aggregate / Correlate for larger analyses. |
| Pagination page | 1000 default | Larger → use streaming. |
| Filter complexity | 50 predicates / filter | Keeps the SQL compiler bounded. |

Exceeding a limit returns `PAYLOAD_TOO_LARGE` (or `RESOURCE_EXHAUSTED` for stream-cumulative limits) with a `metadata.limit` and `metadata.observed` hint.

---

## Service: `OhdcService`

The main service. Every consumer (Connect, Care, MCPs, integrators, sensors) speaks against this surface. Token kind enforced per RPC.

Current `ohdc.v0` service surface:

```protobuf
service OhdcService {
  rpc PutEvents(PutEventsRequest) returns (PutEventsResponse);
  rpc AttachBlob(stream AttachBlobChunk) returns (AttachBlobResponse);
  rpc QueryEvents(QueryEventsRequest) returns (stream Event);
  rpc GetEventByUlid(GetEventByUlidRequest) returns (Event);
  rpc Aggregate(AggregateRequest) returns (AggregateResponse);
  rpc Correlate(CorrelateRequest) returns (CorrelateResponse);
  rpc ReadSamples(ReadSamplesRequest) returns (stream SampleBatch);
  rpc ReadAttachment(ReadAttachmentRequest) returns (stream AttachmentChunk);
  rpc CreateGrant(CreateGrantRequest) returns (CreateGrantResponse);
  rpc ListGrants(ListGrantsRequest) returns (ListGrantsResponse);
  rpc UpdateGrant(UpdateGrantRequest) returns (Grant);
  rpc RevokeGrant(RevokeGrantRequest) returns (RevokeGrantResponse);
  rpc CreateCase(CreateCaseRequest) returns (Case);   // OpenCase in older prose
  rpc UpdateCase(UpdateCaseRequest) returns (Case);
  rpc CloseCase(CloseCaseRequest) returns (Case);
  rpc ReopenCase(ReopenCaseRequest) returns (Case);
  rpc ListCases(ListCasesRequest) returns (ListCasesResponse);
  rpc GetCase(GetCaseRequest) returns (Case);
  rpc AddCaseFilter(AddCaseFilterRequest) returns (CaseFilter);
  rpc RemoveCaseFilter(RemoveCaseFilterRequest) returns (RemoveCaseFilterResponse);
  rpc ListCaseFilters(ListCaseFiltersRequest) returns (ListCaseFiltersResponse);
  rpc AuditQuery(AuditQueryRequest) returns (stream AuditEntry);
  rpc ListPending(ListPendingRequest) returns (ListPendingResponse);
  rpc ApprovePending(ApprovePendingRequest) returns (ApprovePendingResponse);
  rpc RejectPending(RejectPendingRequest) returns (RejectPendingResponse);
  rpc ListPendingQueries(ListPendingQueriesRequest) returns (stream PendingQuery);
  rpc ApprovePendingQuery(ApprovePendingQueryRequest) returns (ApprovePendingQueryResponse);
  rpc RejectPendingQuery(RejectPendingQueryRequest) returns (RejectPendingQueryResponse);
  rpc Export(ExportRequest) returns (stream ExportChunk);
  rpc Import(stream ImportChunk) returns (ImportResponse);
  rpc RegisterSigner(RegisterSignerRequest) returns (RegisterSignerResponse);
  rpc ListSigners(ListSignersRequest) returns (ListSignersResponse);
  rpc RevokeSigner(RevokeSignerRequest) returns (RevokeSignerResponse);
  rpc WhoAmI(WhoAmIRequest) returns (WhoAmIResponse);
  rpc Health(HealthRequest) returns (HealthResponse);
}
```

The Rust core also exposes helpers equivalent to `ForceCloseCase`,
`HandoffCase`, `IssueRetrospectiveGrant`, and `IssueDelegateGrant`; the proto
surface uses the case/grant RPCs above plus typed request flows rather than
separate method names for every helper.

### Writes

#### `PutEvents`

Append one or more events atomically. **Token kinds**: self-session, grant (write-scope), device.

```protobuf
service OhdcService {
  rpc PutEvents(PutEventsRequest) returns (PutEventsResponse);
}

message PutEventsRequest {
  repeated EventInput events = 1;
  // If true, the entire batch is rejected on any per-event validation failure.
  // If false (default), invalid events are reported in the response and valid events commit.
  bool atomic = 2;
}

message PutEventsResponse {
  // One per input event, in the same order.
  repeated PutEventResult results = 1;
}

message PutEventResult {
  oneof outcome {
    PutEventCommitted committed = 1;
    PutEventPending pending = 2;          // routed through the approval queue
    ErrorInfo error = 3;                  // validation or scope rejection for this specific event
  }
}

message PutEventCommitted {
  Ulid ulid = 1;
  int64 committed_at_ms = 2;
}

message PutEventPending {
  Ulid ulid = 1;                          // ULID is allocated even for pending events
  int64 expires_at_ms = 2;                // when the pending entry auto-expires
}
```

**Semantics:**

- All events validated against the registry per `storage-format.md` "Validation on write."
- Token-kind-specific scope check. Grants check `grant_write_event_type_rules`.
- Grant tokens with `approval_mode != never_required` route to `pending_events`; the user is notified per `notifications.md`.
- Idempotency via `(source, source_id)` per "Idempotency" above.
- Audit row per event: `actor_type` matches token kind.

**Errors**: `UNKNOWN_TYPE`, `UNKNOWN_CHANNEL`, `WRONG_VALUE_TYPE`, `INVALID_UNIT`, `INVALID_ENUM`, `MISSING_REQUIRED_CHANNEL`, `INVALID_TIMESTAMP`, `OUT_OF_SCOPE` (write scope or backdating window), `IDEMPOTENCY_CONFLICT`, `PAYLOAD_TOO_LARGE`.

#### `AttachBlob`

Upload a sidecar attachment (raw ECG, image, PDF). **Token kinds**: same as `PutEvents`. Returns an `AttachmentRef` whose `ulid` can be referenced from a subsequent `EventInput.attachment_ulids`.

```protobuf
rpc AttachBlob(stream AttachBlobChunk) returns (AttachBlobResponse);

message AttachBlobResponse {
  AttachmentRef attachment = 1;
}
```

Streaming framing per "Streaming patterns" above. SHA-256 verified server-side; mismatch → `INVALID_ARGUMENT`.

### Reads

#### `QueryEvents`

Iterate events matching a filter, server-streaming. **Token kinds**: self-session, grant (read-scope).

```protobuf
rpc QueryEvents(QueryEventsRequest) returns (stream Event);

message QueryEventsRequest {
  EventFilter filter = 1;
}
```

**Semantics:**

- Resolution algorithm runs per [`storage-format.md`](storage-format.md). Stripped channels are omitted; rejected events count into `audit_log.rows_filtered`.
- Stream order is per `EventFilter.sort` (default `TIME_DESC`).
- Cancellation / errors per "Streaming patterns."

**Errors**: `OUT_OF_SCOPE` (filter conflicts with grant scope before any event match), `INVALID_FILTER`, `RATE_LIMITED`.

#### `GetEventByUlid`

Single-event lookup. **Token kinds**: self-session, grant. Returns `Event` or `NOT_FOUND`. Out-of-scope events return `NOT_FOUND` (not `OUT_OF_SCOPE`) to avoid leaking existence to grantees.

```protobuf
rpc GetEventByUlid(GetEventByUlidRequest) returns (Event);

message GetEventByUlidRequest {
  Ulid ulid = 1;
}
```

#### `Aggregate`

Numeric aggregates over a channel, bucketed by time. **Token kinds**: self-session, grant. Compatible with `aggregation_only=1` grants.

```protobuf
rpc Aggregate(AggregateRequest) returns (AggregateResponse);

message AggregateRequest {
  string channel_path = 1;
  EventFilter filter = 2;                 // type filter required to disambiguate
  AggregateOp op = 3;
  Bucket bucket = 4;
}

enum AggregateOp {
  AVG = 0;
  SUM = 1;
  MIN = 2;
  MAX = 3;
  COUNT = 4;
  MEDIAN = 5;
  P95 = 6;
  P99 = 7;
  STDDEV = 8;
}

message Bucket {
  oneof bucket {
    google.protobuf.Duration fixed = 1;   // e.g. 15 min, 1 hour, 1 day
    CalendarBucket calendar = 2;          // e.g. by-calendar-day in user's TZ
  }
}

message CalendarBucket {
  // Bucket boundaries in this TZ (IANA name). Empty = UTC.
  string tz_name = 1;
  CalendarUnit unit = 2;
}

enum CalendarUnit {
  HOUR = 0;
  DAY = 1;
  WEEK = 2;
  MONTH = 3;
  YEAR = 4;
}

message AggregateResponse {
  repeated AggregateBucketResult buckets = 1;
}

message AggregateBucketResult {
  int64 bucket_start_ms = 1;
  int64 bucket_end_ms = 2;
  int64 sample_count = 3;
  double value = 4;                       // result of the op
}
```

#### `Correlate`

Temporal correlation between two channels (or two event types). For the patient-side LLM and for clinical analysis tools.

```protobuf
rpc Correlate(CorrelateRequest) returns (CorrelateResponse);

message CorrelateRequest {
  // Each side: either an event type (counts/aggregates events of that type) or a channel (timeline of values).
  CorrelateSide a = 1;
  CorrelateSide b = 2;
  // Window: for each occurrence of A, look for B in [a_time, a_time + window].
  google.protobuf.Duration window = 3;
  EventFilter scope = 4;                  // filter the time/source range
}

message CorrelateSide {
  oneof spec {
    string event_type = 1;                // e.g. "std.meal"
    string channel_path = 2;              // e.g. "std.glucose.value"
  }
}

message CorrelateResponse {
  repeated CorrelatePair pairs = 1;
  CorrelateStats stats = 2;
}

message CorrelatePair {
  Ulid a_ulid = 1;
  int64 a_time_ms = 2;
  // For each match in window:
  repeated CorrelateMatch matches = 3;
}

message CorrelateMatch {
  Ulid b_ulid = 1;
  int64 b_time_ms = 2;
  optional double b_value = 3;            // populated when b is a channel
}

message CorrelateStats {
  int64 a_count = 1;
  int64 b_count = 2;
  int64 paired_count = 3;
  optional double mean_b_value = 4;
  optional double mean_lag_ms = 5;
}
```

`Correlate` is intentionally simple — for richer statistical analyses use the export and run them client-side. The server-side version exists so the resolver can apply grants once across the whole computation; client-side correlation requires the client to fetch raw events first, which violates `aggregation_only`.

#### `ReadSamples`

Decoded sample stream from one or more dense series events. **Token kinds**: self-session, grant.

```protobuf
rpc ReadSamples(ReadSamplesRequest) returns (stream SampleBatch);

message ReadSamplesRequest {
  Ulid event_ulid = 1;
  string channel_path = 2;
  // Subset of the event's full time range. Optional; empty = the whole event.
  optional int64 from_ms = 3;
  optional int64 to_ms = 4;
  // Server-side downsample: if set, the server returns at most this many samples,
  // averaging within buckets. 0 = no downsample, raw samples.
  int32 max_samples = 5;
}

message SampleBatch {
  repeated Sample samples = 1;
}
```

Blocked when grant has `aggregation_only=1` → `OUT_OF_SCOPE`.

#### `ReadAttachment`

Streamed sidecar blob download. **Token kinds**: self-session, grant.

```protobuf
rpc ReadAttachment(ReadAttachmentRequest) returns (stream AttachmentChunk);

message ReadAttachmentRequest {
  Ulid attachment_ulid = 1;
}

message AttachmentChunk {
  oneof content {
    AttachmentInit init = 1;              // first frame: metadata
    bytes data = 2;
    AttachmentFinish finish = 3;
  }
}

message AttachmentInit {
  AttachmentRef ref = 1;
}

message AttachmentFinish {
  bytes expected_sha256 = 1;
}
```

### Grants

`CreateGrant`, `ListGrants`, `UpdateGrant`, `RevokeGrant`. **Token kind**: self-session only — only the user can manage their own grants.

```protobuf
rpc CreateGrant(CreateGrantRequest) returns (CreateGrantResponse);

message CreateGrantRequest {
  string grantee_label = 1;
  string grantee_kind = 2;                // 'human'|'app'|'service'|'emergency'|'device'|'delegate'
  optional Ulid grantee_ulid = 3;
  optional string purpose = 4;

  // Read scope policy
  string default_action = 5;              // 'allow' | 'deny'
  bool aggregation_only = 6;
  bool strip_notes = 7;
  bool require_approval_per_query = 8;
  optional int32 rolling_window_days = 9;
  optional TimeWindow absolute_window = 10;
  repeated GrantEventTypeRule event_type_rules = 11;
  repeated GrantChannelRule channel_rules = 12;
  repeated GrantSensitivityRule sensitivity_rules = 13;

  // Write scope policy
  string approval_mode = 14;              // 'always' | 'auto_for_event_types' | 'never_required'
  repeated GrantWriteEventTypeRule write_event_type_rules = 15;
  repeated string auto_approve_event_types = 16;

  // Lifecycle / general
  optional int64 expires_at_ms = 17;
  bool notify_on_access = 18;
  optional int32 max_queries_per_day = 19;
  optional int32 max_queries_per_hour = 20;

  // Optional case bindings. Empty = open-scope grant. >=1 = case-bound grant
  // whose read scope is the union of these cases' scopes (per the case scope
  // resolution algorithm in storage-format.md), intersected with the access
  // rules above. Cases must already exist; reference by ULID.
  repeated Ulid case_ulids = 21;
}

message TimeWindow { int64 from_ms = 1; int64 to_ms = 2; }
message GrantEventTypeRule { string event_type = 1; string effect = 2; }      // 'allow' | 'deny'
message GrantChannelRule { string channel_path = 1; string effect = 2; }
message GrantSensitivityRule { string sensitivity_class = 1; string effect = 2; }
message GrantWriteEventTypeRule { string event_type = 1; string effect = 2; }

message CreateGrantResponse {
  Grant grant = 1;
  string token = 2;                       // ohdg_... — the bearer token; shown to the user once
  // Share URL bundles token + storage URL + cert pin + (optionally) cases.
  // Format: ohd://grant/<token>?storage=<url>&pin=<sha256_spki_b64url>[&cases=<ulid>,<ulid>...]
  string share_url = 3;
  bytes share_qr_png = 4;                 // pre-rendered QR encoding share_url
}

message Grant {
  Ulid ulid = 1;
  string grantee_label = 2;
  string grantee_kind = 3;
  optional Ulid grantee_ulid = 4;
  optional string purpose = 5;
  int64 created_at_ms = 6;
  optional int64 expires_at_ms = 7;
  optional int64 revoked_at_ms = 8;

  // Same policy fields as CreateGrantRequest, less the token.
  string default_action = 9;
  bool aggregation_only = 10;
  bool strip_notes = 11;
  bool require_approval_per_query = 12;
  optional int32 rolling_window_days = 13;
  optional TimeWindow absolute_window = 14;
  repeated GrantEventTypeRule event_type_rules = 15;
  repeated GrantChannelRule channel_rules = 16;
  repeated GrantSensitivityRule sensitivity_rules = 17;

  string approval_mode = 18;
  repeated GrantWriteEventTypeRule write_event_type_rules = 19;
  repeated string auto_approve_event_types = 20;

  bool notify_on_access = 21;
  optional int32 max_queries_per_day = 22;
  optional int32 max_queries_per_hour = 23;

  // Case bindings. Empty = open-scope. Many-to-many via grant_cases internally.
  repeated Ulid case_ulids = 24;

  // Operational stats (populated on read, not write):
  optional int64 last_used_ms = 25;
  int64 use_count = 26;
}

rpc ListGrants(ListGrantsRequest) returns (ListGrantsResponse);

message ListGrantsRequest {
  optional bool include_revoked = 1;       // default false
  optional bool include_expired = 2;       // default false
  optional string grantee_kind = 3;        // filter by kind
  PageRequest page = 4;
}

message ListGrantsResponse {
  repeated Grant grants = 1;
  PageResponse page = 2;
}

rpc UpdateGrant(UpdateGrantRequest) returns (Grant);

message UpdateGrantRequest {
  Ulid grant_ulid = 1;
  // Sparse update — fields present are updated; absent are unchanged.
  // Updates allowed: label, expires_at_ms, all rule lists, all policy bools, rate limits.
  // Updates NOT allowed: grantee_kind, grantee_ulid, default_action (revoke + re-create instead),
  // approval_mode (revoke + re-create — changing this surprises the grantee mid-relationship).
  optional string grantee_label = 2;
  optional int64 expires_at_ms = 3;
  // ... mirrors of mutable Grant fields ...
}

rpc RevokeGrant(RevokeGrantRequest) returns (RevokeGrantResponse);

message RevokeGrantRequest {
  Ulid grant_ulid = 1;
  optional string reason = 2;             // for the user's own audit; not surfaced to the grantee
}

message RevokeGrantResponse {
  int64 revoked_at_ms = 1;
}
```

`CreateGrant` is **synchronous** against the primary (per [`privacy-access.md`](privacy-access.md) "Revocation semantics") — even on a cache instance, the call goes to the primary and either commits there or fails loudly.

**No grant chaining.** `CreateGrant`, `UpdateGrant`, and `RevokeGrant` all require a self-session token. Calling them with a grant token or device token returns `WRONG_TOKEN_KIND`. This is the structural enforcement of the "grants don't chain" invariant from [`privacy-access.md`](privacy-access.md): the user is the sole source of grants; grantees cannot delegate or sub-issue.

### Cases

```protobuf
service OhdcService {
  rpc CreateCase(CreateCaseRequest) returns (Case);
  rpc UpdateCase(UpdateCaseRequest) returns (Case);
  rpc CloseCase(CloseCaseRequest) returns (Case);
  rpc ReopenCase(ReopenCaseRequest) returns (Case);
  rpc ListCases(ListCasesRequest) returns (ListCasesResponse);
  rpc GetCase(GetCaseRequest) returns (Case);

  rpc AddCaseFilter(AddCaseFilterRequest) returns (CaseFilter);
  rpc RemoveCaseFilter(RemoveCaseFilterRequest) returns (RemoveCaseFilterResponse);
  rpc ListCaseFilters(ListCaseFiltersRequest) returns (ListCaseFiltersResponse);
}

message Case {
  Ulid ulid = 1;
  string case_type = 2;                  // 'emergency'|'admission'|'visit'|'cycle'|...
  optional string case_label = 3;
  int64 started_at_ms = 4;
  optional int64 ended_at_ms = 5;
  optional Ulid parent_case_ulid = 6;
  optional Ulid predecessor_case_ulid = 7;
  optional Ulid opening_authority_grant_ulid = 8;
  optional int32 inactivity_close_after_h = 9;
  int64 last_activity_at_ms = 10;
}

message CreateCaseRequest {
  string case_type = 1;
  optional string case_label = 2;
  optional Ulid parent_case_ulid = 3;
  optional Ulid predecessor_case_ulid = 4;
  optional int32 inactivity_close_after_h = 5;
  // Optional: initial filters to add atomically with the case.
  repeated EventFilter initial_filters = 6;
}

message UpdateCaseRequest {
  Ulid case_ulid = 1;
  optional string case_label = 2;
  optional Ulid parent_case_ulid = 3;     // re-parent (validated against cycles)
  optional Ulid predecessor_case_ulid = 4;
  optional int32 inactivity_close_after_h = 5;
}

message CloseCaseRequest {
  Ulid case_ulid = 1;
  optional string reason = 2;
}

message ReopenCaseRequest {
  oneof method {
    Ulid case_reopen_token_ulid = 1;     // for authority-side reopen via token
    PatientReopen patient = 2;            // for patient-side force-reopen (self-session only)
  }
}

message PatientReopen {
  Ulid case_ulid = 1;
}

message ListCasesRequest {
  optional bool include_closed = 1;
  optional string case_type = 2;
  PageRequest page = 3;
}

message ListCasesResponse {
  repeated Case cases = 1;
  PageResponse page = 2;
}

message GetCaseRequest {
  Ulid case_ulid = 1;
}

message CaseFilter {
  Ulid ulid = 1;
  Ulid case_ulid = 2;
  EventFilter filter = 3;
  optional string filter_label = 4;
  int64 added_at_ms = 5;
  optional Ulid added_by_grant_ulid = 6;
}

message AddCaseFilterRequest {
  Ulid case_ulid = 1;
  EventFilter filter = 2;
  optional string filter_label = 3;
}

message RemoveCaseFilterRequest {
  Ulid case_filter_ulid = 1;
}

message RemoveCaseFilterResponse {
  int64 removed_at_ms = 1;
}

message ListCaseFiltersRequest {
  Ulid case_ulid = 1;
  bool include_removed = 2;
}

message ListCaseFiltersResponse {
  repeated CaseFilter filters = 1;
}
```

**Token-kind rules** for case operations:

- `CreateCase`, `UpdateCase`, `CloseCase`, `ReopenCase` (patient method), `AddCaseFilter`, `RemoveCaseFilter` — **self-session only** (the user owns case configuration; authority-opened cases happen automatically inside break-glass flow, not via these RPCs).
- `ReopenCase` (token method) — accepts an authority's case-reopen-token.
- `ListCases`, `GetCase`, `ListCaseFilters` — self-session sees all; grant tokens see only cases referenced by their `grant_cases` rows; device tokens cannot list cases.

`CloseCase` is also called automatically by:
- The auto-close inactivity sweep (no RPC; storage-internal).
- Handoff (when an authority hands off, the predecessor case is closed; the new case is opened with `predecessor_case_ulid` set).

`AddCaseFilter` typical patterns:
- Patient curating a case: `EventFilter { event_ulids_in: [u1, u2, ...] }`.
- Emergency case: `EventFilter { from_ms, to_ms, device_id_in: [responder_device] }` — added at break-glass time.
- Cycle / chronic-condition case: `EventFilter { event_types_in: ['std.menstrual_*'] }` plus optional time bounds.

### Audit

```protobuf
rpc AuditQuery(AuditQueryRequest) returns (stream AuditEntry);

message AuditQueryRequest {
  optional int64 from_ms = 1;
  optional int64 to_ms = 2;
  optional Ulid grant_ulid = 3;           // only entries for this grant
  optional string actor_type = 4;         // 'self' | 'grant' | 'system'
  optional string action = 5;             // 'read' | 'write' | 'grant_create' | ...
  optional string result = 6;             // 'success' | 'partial' | 'rejected' | 'error'

  // If true, after the historical results are streamed, the server keeps the stream open
  // and pushes new audit rows as they appear. Useful for live dashboards.
  bool tail = 7;
}

message AuditEntry {
  int64 ts_ms = 1;
  string actor_type = 2;
  optional Ulid grant_ulid = 3;
  string action = 4;
  string query_kind = 5;
  // Canonical request payload (Protobuf-JSON of the original request); for forensics.
  string query_params_json = 6;
  optional int64 rows_returned = 7;
  optional int64 rows_filtered = 8;
  string result = 9;
  optional string reason = 10;
  optional string caller_ip = 11;
  optional string caller_ua = 12;
}
```

**Token kinds**: self-session sees full audit; grants see only their own audit (filtered server-side).

### Pending events (write-with-approval)

```protobuf
rpc ListPending(ListPendingRequest) returns (ListPendingResponse);

message ListPendingRequest {
  optional Ulid submitting_grant_ulid = 1;  // grant tokens may scope to their own
  optional string status = 2;               // 'pending' | 'approved' | 'rejected' | 'expired'
  PageRequest page = 3;
}

message ListPendingResponse {
  repeated PendingEvent pending = 1;
  PageResponse page = 2;
}

message PendingEvent {
  Ulid ulid = 1;
  int64 submitted_at_ms = 2;
  Ulid submitting_grant_ulid = 3;
  // The full canonical event payload; same shape that EventInput would have.
  Event event = 4;
  string status = 5;                        // 'pending' | 'approved' | 'rejected' | 'expired'
  optional int64 reviewed_at_ms = 6;
  optional string rejection_reason = 7;
  int64 expires_at_ms = 8;
  optional Ulid approved_event_ulid = 9;
}

rpc ApprovePending(ApprovePendingRequest) returns (ApprovePendingResponse);

message ApprovePendingRequest {
  Ulid pending_ulid = 1;
  // If true, also adds the event_type to the submitting grant's auto-approve list.
  bool also_auto_approve_this_type = 2;
}

message ApprovePendingResponse {
  Ulid event_ulid = 1;                      // same as the pending ulid
  int64 committed_at_ms = 2;
}

rpc RejectPending(RejectPendingRequest) returns (RejectPendingResponse);

message RejectPendingRequest {
  Ulid pending_ulid = 1;
  optional string reason = 2;
}

message RejectPendingResponse {
  int64 rejected_at_ms = 1;
}
```

**Token kinds**: self-session for approve/reject; grants can list their own pending submissions.

### Pending queries (read-with-approval)

Grants with `require_approval_per_query=1` enqueue reads in `pending_queries`
instead of executing immediately.

```protobuf
rpc ListPendingQueries(ListPendingQueriesRequest) returns (stream PendingQuery);
rpc ApprovePendingQuery(ApprovePendingQueryRequest) returns (ApprovePendingQueryResponse);
rpc RejectPendingQuery(RejectPendingQueryRequest) returns (RejectPendingQueryResponse);
```

Self-session tokens can list and decide pending read requests. Grant tokens
receive a pending/approval error until the user approves the queued query.

### Source signing

High-trust sources submit `EventInput.source_signature`. Storage verifies the
signature over canonical CBOR event bytes using the registered signer row and
rejects invalid signatures before commit. Supported `sig_alg` values are
`ed25519`, `rs256`, and `es256`.

```protobuf
rpc RegisterSigner(RegisterSignerRequest) returns (RegisterSignerResponse);
rpc ListSigners(ListSignersRequest) returns (ListSignersResponse);
rpc RevokeSigner(RevokeSignerRequest) returns (RevokeSignerResponse);
```

These signer registry RPCs are self-session only. Queried events that were
accepted with a valid source signature return `Event.signed_by`.

### Export / Import

```protobuf
rpc Export(ExportRequest) returns (stream ExportChunk);

message ExportRequest {
  // Optional time bounds. Empty = whole file.
  optional int64 from_ms = 1;
  optional int64 to_ms = 2;
  // Optional inclusion list. Empty = everything.
  repeated string include_event_types = 3;
  // Optional encryption with a recipient passphrase. Empty = plaintext export.
  optional string encrypt_to_passphrase = 4;
  // Resumability — if set, server picks up at this position. Returned in earlier ExportChunk.
  optional string resume_token = 5;
}

message ExportChunk {
  oneof content {
    ExportInit init = 1;                  // first chunk
    ExportFrame frame = 2;                // an event / grant / audit row, etc.
    ExportFinish finish = 3;
  }
}

// Init carries the export manifest: format version, source instance identity key,
// signature, encryption parameters if encrypted.
message ExportInit { ... }

message ExportFrame {
  oneof entity {
    Event event = 1;
    Grant grant = 2;
    AuditEntry audit_entry = 3;
    AttachmentBlob attachment = 4;
    PendingEvent pending = 5;
    DeviceRow device = 6;
    AppVersionRow app_version = 7;
    RegistryEntry registry_entry = 8;
    PeerSyncRow peer_sync = 9;
  }
}

message ExportFinish {
  string resume_token = 1;                // for any subsequent partial re-fetch
  bytes signature = 2;                    // Ed25519 signature over the whole export
  string source_instance_pubkey_hex = 3;
}

rpc Import(stream ImportChunk) returns (ImportResponse);

message ImportChunk {
  oneof content {
    ImportInit init = 1;
    ExportFrame frame = 2;
    ImportFinish finish = 3;
  }
}

message ImportResponse {
  int64 events_imported = 1;
  int64 grants_imported = 2;
  int64 audit_entries_imported = 3;
  repeated string warnings = 4;
  repeated UnknownExtension unknown_extensions = 5;
}

message UnknownExtension {
  string namespace = 1;
  string preserved_as = 2;                // 'metadata._imported_extensions'
  int64 entries = 3;
}
```

**Token kind**: self-session only.

### Diagnostics

```protobuf
rpc WhoAmI(WhoAmIRequest) returns (WhoAmIResponse);

message WhoAmIRequest {}

message WhoAmIResponse {
  Ulid user_ulid = 1;
  string token_kind = 2;                  // 'self_session' | 'grant' | 'device'
  optional Ulid grant_ulid = 3;           // if grant or device
  optional string grantee_label = 4;
  // Effective scope summary, populated for grant/device tokens.
  optional Grant effective_grant = 5;
  // Server's view of the caller for sanity checks.
  string caller_ip = 6;
  optional string device_label = 7;
}

rpc Health(HealthRequest) returns (HealthResponse);

message HealthRequest {}

message HealthResponse {
  string status = 1;                      // 'ok' | 'degraded' | 'down'
  int64 server_time_ms = 2;
  string server_version = 3;
  string protocol_version = 4;            // e.g. "ohdc.v0"
  optional int32 registry_version = 5;
  // Per-subsystem state:
  map<string, string> subsystems = 6;     // e.g. { "system_db": "ok", "blobs": "ok", "relay": "ok" }
}
```

`Health` is unauthenticated (the only OHDC RPC that is). Used by load balancers, Docker healthchecks, external monitors.

`WhoAmI` requires any valid token; helps integrators sanity-check that their token is being parsed as expected.

---

## Service: `AuthService`

Self-session-side identity and session operations. Distinct from `OhdcService` so its scope is clear.

```protobuf
service AuthService {
  rpc ListIdentities(ListIdentitiesRequest) returns (ListIdentitiesResponse);
  rpc LinkIdentityStart(LinkIdentityStartRequest) returns (LinkIdentityStartResponse);
  rpc CompleteIdentityLink(CompleteIdentityLinkRequest) returns (CompleteIdentityLinkResponse);
  rpc UnlinkIdentity(UnlinkIdentityRequest) returns (UnlinkIdentityResponse);
  rpc SetPrimaryIdentity(SetPrimaryIdentityRequest) returns (SetPrimaryIdentityResponse);

  rpc ListSessions(ListSessionsRequest) returns (ListSessionsResponse);
  rpc RevokeSession(RevokeSessionRequest) returns (RevokeSessionResponse);
  rpc Logout(LogoutRequest) returns (LogoutResponse);
  rpc LogoutEverywhere(LogoutEverywhereRequest) returns (LogoutEverywhereResponse);

  rpc IssueInvite(IssueInviteRequest) returns (IssueInviteResponse);
  rpc ListInvites(ListInvitesRequest) returns (ListInvitesResponse);
  rpc RevokeInvite(RevokeInviteRequest) returns (RevokeInviteResponse);

  rpc IssueDeviceToken(IssueDeviceTokenRequest) returns (IssueDeviceTokenResponse);

  rpc RegisterPushToken(RegisterPushTokenRequest) returns (RegisterPushTokenResponse);
  rpc UpdateNotificationPreferences(UpdateNotificationPreferencesRequest) returns (UpdateNotificationPreferencesResponse);
}
```

Each request/response message is mostly self-explanatory by name + the surrounding spec — see [`auth.md`](auth.md), [`care-auth.md`](care-auth.md), and [`notifications.md`](notifications.md) for the semantics. Highlights:

- `LinkIdentityStart` returns a one-time URL the user opens to complete the OAuth flow against a new provider; on completion the new identity is bound to the user's existing `user_ulid`.
- `CompleteIdentityLink` verifies the provider `id_token` and commits the pending identity link.
- `SetPrimaryIdentity` promotes one linked identity to the primary login identity.
- `IssueDeviceToken` is the in-app Model 3 path from the deferred device-pairing design — the user's own apps (Connect's bridge service) request a per-bridge token under self-session. Fully usable in v0 even though the broader device-pairing UX is deferred.
- `RegisterPushToken` is the entry point for the notification system; idempotent on `(platform, token)`.

---

## Service: `SyncService`

Cache-to-primary replication uses a separate self-session-only service:

```protobuf
service SyncService {
  rpc Hello(HelloRequest) returns (HelloResponse);
  rpc PushFrames(stream PushFrame) returns (stream PushAck);
  rpc PullFrames(PullRequest) returns (stream PushFrame);
  rpc PushAttachmentBlob(stream AttachmentChunk) returns (AttachmentAck);
  rpc PullAttachmentBlob(PullAttachmentRequest) returns (stream AttachmentChunk);
  rpc CreateGrantOnPrimary(CreateGrantRequest) returns (CreateGrantResponse);
  rpc RevokeGrantOnPrimary(RevokeGrantRequest) returns (RevokeGrantResponse);
  rpc UpdateGrantOnPrimary(UpdateGrantRequest) returns (Grant);
}
```

`PushAttachmentBlob` and `PullAttachmentBlob` carry encrypted sidecar payloads
outside the event-frame stream. Grant lifecycle remains RPC-gated against the
primary rather than replicated as eventually-consistent frames.

### Token-kind matrix

| RPC | self-session | grant | device |
|---|---|---|---|
| OhdcService.PutEvents | ✅ | ✅ (write scope) | ✅ |
| OhdcService.QueryEvents | ✅ | ✅ (read scope) | ❌ |
| OhdcService.GetEventByUlid | ✅ | ✅ | ❌ |
| OhdcService.Aggregate / Correlate | ✅ | ✅ | ❌ |
| OhdcService.ReadSamples / ReadAttachment | ✅ | ✅ (unless `aggregation_only`) | ❌ |
| OhdcService.AttachBlob | ✅ | ✅ | ✅ |
| OhdcService.CreateGrant / ListGrants / UpdateGrant / RevokeGrant | ✅ | ❌ | ❌ |
| OhdcService.CreateCase / UpdateCase / CloseCase / AddCaseFilter / RemoveCaseFilter | ✅ | ❌ | ❌ |
| OhdcService.ReopenCase (token method) | n/a | ✅ (with case_reopen_token) | ❌ |
| OhdcService.ListCases / GetCase / ListCaseFilters | ✅ (all) | ✅ (own only) | ❌ |
| OhdcService.AuditQuery | ✅ (full) | ✅ (own only) | ❌ |
| OhdcService.ListPending | ✅ | ✅ (own submissions) | ❌ |
| OhdcService.ApprovePending / RejectPending | ✅ | ❌ | ❌ |
| OhdcService.Export / Import | ✅ | ❌ | ❌ |
| OhdcService.WhoAmI | ✅ | ✅ | ✅ |
| OhdcService.Health | (no auth) | (no auth) | (no auth) |
| AuthService.* (most) | ✅ | ❌ | ❌ |
| AuthService.IssueDeviceToken | ✅ | ❌ | ❌ |

`WRONG_TOKEN_KIND` returned for mismatches.

---

## Service: `RelayService`

Storage-side operations against a relay. Used by the storage process itself, not by external clients. **Token kind**: a special "storage registration" credential established at first registration (separate from the user's tokens; identifies the storage to the relay).

```protobuf
service RelayService {
  rpc Register(RegisterRequest) returns (RegisterResponse);
  rpc RefreshRegistration(RefreshRegistrationRequest) returns (RefreshRegistrationResponse);
  rpc Heartbeat(HeartbeatRequest) returns (HeartbeatResponse);
  rpc Deregister(DeregisterRequest) returns (DeregisterResponse);
  rpc OpenTunnel(stream TunnelFrame) returns (stream TunnelFrame);
}
```

`OpenTunnel` is the long-lived bidirectional stream the storage maintains while reachable. Frames carry opaque ciphertext from external consumers (the relay forwards them). Storage's responses go back the other way. Per [`../components/relay.md`](../components/relay.md) "Persistence."

The deeper Relay protocol details (TLS-through-tunnel cert/identity model) are open per Task #11; this service surface is settled.

---

## HTTP-only endpoints (OAuth + discovery)

Not Connect-RPC — standard HTTP, served alongside the RPC services on the same domain. Per [`auth.md`](auth.md) "Putting it together":

| Path | Method | Purpose |
|---|---|---|
| `/.well-known/oauth-authorization-server` | GET | RFC 8414 metadata |
| `/.well-known/openid-configuration` | GET | OIDC discovery (mirrors above; many libraries look here) |
| `/authorize` | GET | OAuth Authorization endpoint; renders OHD's login page |
| `/oidc-callback` | GET | OIDC callback receiver from upstream provider |
| `/token` | POST | OAuth token endpoint (auth_code, refresh_token, device_code) |
| `/device` | GET, POST | OAuth Device Authorization Grant user-confirmation page |
| `/oauth/register` | POST | RFC 7591 dynamic client registration |
| `/health` | GET | Same as `OhdcService.Health` for non-RPC monitors |
| `/metrics` | GET | Prometheus exposition; restricted at network layer |

These are part of the OHDC v0 contract — every implementation exposes them at the canonical paths.

---

## Versioning

The `.proto` package name carries the version: `ohdc.v0`. While the protocol
is in v0, additive changes are allowed but must be documented here and in the
canonical proto files:

- **Additive changes** (new optional fields, new RPCs, new error codes, new event-type catalog entries) are non-breaking. Old clients keep working; new fields are silently ignored on older readers.
- **Renames or behavioral changes** require either an additive shadow field with deprecation of the old, or a major bump.
- **Post-v0 major bumps** (`ohdc.vN`) are new packages; multiple versions can be served by the same storage during a migration window. The conformance corpus carries fixtures for each supported version.

`Health.protocol_version` reports the current version. Clients call `Health` once at startup; if they want a version they can't speak, they fail-fast with a clear "your client is too old / too new" message.

Buf's `breaking` lint runs in CI on the `ohd-protocol` repo and refuses commits that would break wire compatibility within a major version.

---

## Conformance

Implementations claiming OHDC v0 conformance must:

1. Compile and serve the canonical `.proto` files unchanged (no field deletion, no renumbering, no semantic drift).
2. Pass the conformance corpus (Task #16) end-to-end: input event sequence → expected query outputs → byte-equal sample blocks → grant resolution fixtures.
3. Honor every error code in this doc with the correct HTTP status mapping.
4. Implement every RPC in the token-kind matrix above with the correct scope behavior.
5. Expose the HTTP-only endpoints at the canonical paths.

Vendors may add custom RPCs to `com.<vendor>.ohdc_ext.v0.*` namespaces; those are out of scope for v0 conformance and don't claim it.

---

## Cross-references

- High-level overview: [`../components/connect.md`](../components/connect.md) "Wire format", "Operations"
- On-disk model these messages serialize to/from: [`storage-format.md`](storage-format.md)
- Token kinds and OAuth: [`auth.md`](auth.md)
- Care-side operator concerns: [`care-auth.md`](care-auth.md)
- Encryption: [`encryption.md`](encryption.md)
- Notifications: [`notifications.md`](notifications.md)
- Relay protocol: [`../components/relay.md`](../components/relay.md)
- Vocabulary catalog (event types and channels): [`data-model.md`](data-model.md)

## Open items (forwarded)

- **Sync wire protocol** — the cache↔primary replication protocol layered on top of OHDC. Spec lives in Task #10; depends on this protocol surface but doesn't change it.
- **Relay TLS-through-tunnel cert/identity model** — Task #11. The `RelayService.OpenTunnel` frame shape and the storage's self-signed cert model are still pinned at the wire level.
- **Conformance corpus** — Task #16. The set of canonical `.proto` files plus input/output fixtures lives in the `ohd-protocol` repo when written.
- **Operator-side admin RPCs** — invite-management for `invite_only` mode beyond what the user can issue; deployment configuration; tenant management for multi-tenant deployments. Belongs in a separate `OperatorService` later; not v1.
- **Subscription RPCs for live updates** — beyond `AuditQuery.tail`. If clients need live event streams (Connect web rendering live charts), a `WatchEvents(filter)` server-streaming RPC could be added additively in a v1.x.
