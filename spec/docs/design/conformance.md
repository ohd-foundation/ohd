# Design: OHDC Conformance Corpus

> The body of test fixtures that define what "OHDC v1 conformance" means in practice. Every implementation that claims OHDC v1 (the reference Rust core, future re-implementations, third-party servers) must pass it.
>
> This doc specifies what the corpus *is*, what it *covers*, where it *lives*, and how to *run it*. The actual fixture data lives in the `openhealth-data/ohd-protocol` repo (per [`../roadmap.md`](../roadmap.md)) and is generated alongside the reference implementation.

## Why conformance matters for OHD

OHDC promises a portable, multi-implementation protocol. If two servers claim conformance but disagree on (say) how an enum ordinal renders after a registry alias resolves, the portability promise breaks at the seam. Writing the spec carefully isn't enough — there have to be **executable, byte-precise fixtures** that catch drift the moment it happens.

Other ecosystems with similar shape (FHIR, SMART, OAuth) all rely on conformance suites for the same reason. OHD adopts the pattern.

## What the corpus covers

Five categories:

### 1. On-disk format conformance

A library claiming OHDC v1 must produce on-disk bytes that other v1 implementations can open. The corpus exercises:

- **Schema migration**: open a `_meta.format_version=1.0` file with no aliases; open the same file with type and channel aliases; open a file post-compaction (aliases resolved, dangling entries removed). Round-trip checks each.
- **Sample-block byte equality**: given a fixed input sample series and codec parameters, the encoded block bytes must be **byte-identical** across implementations. Encoder determinism is part of the spec; this is where it's enforced. Fixtures: 50+ sample series across both encodings (1 and 2) with known input → expected output bytes.
- **ULID generation**: given a fixed timestamp and CSPRNG seed, the ULID's wire form is deterministic. Fixtures cover post-1970 events, pre-1970 (clamped) events, and the at-Unix-epoch boundary case.
- **Channel-tree resolution**: a sequence of registry-add operations followed by a query produces a known set of (event_type_id, channel_id) tuples. Catches subtle off-by-one errors in alias resolution.
- **Encryption parameters**: SQLCipher KDF parameters in `_meta.cipher_kdf` round-trip correctly; opening a file written by one implementation with a known passphrase succeeds in another.

### 2. OHDC RPC conformance

Each RPC in [`ohdc-protocol.md`](ohdc-protocol.md) has fixtures covering:

- **Happy-path request/response**: e.g., `PutEvents` with N events → expected `PutEventsResponse` (ULIDs allocated; committed_at_ms within ε of "now").
- **Error responses**: every error code from the catalog (`UNKNOWN_TYPE`, `INVALID_UNIT`, `OUT_OF_SCOPE`, etc.) has a fixture: input that should produce the error → expected HTTP status + ErrorInfo code.
- **Encoding equivalence**: same logical request encoded as `application/proto` vs `application/json` produces the same response (modulo the Content-Type). Exercises both wire encodings.
- **Idempotency**: re-submit a `(source, source_id)` pair with identical content → no-op; with different content → `IDEMPOTENCY_CONFLICT`.
- **Pagination**: a known input dataset paginated at limit=10 across 5 pages produces the same total set of items in the same order.
- **Filter language**: each `EventFilter` predicate type (`channels[].real_range`, `event_types_in`, `case_ids_in`, etc.) against a known dataset produces a known result set.
- **Streaming framing**: server-streaming RPCs (`QueryEvents`, `ReadSamples`, `Export`, `AuditQuery`) emit the expected sequence of frames; chunked client-streaming RPCs (`AttachBlob`, `Import`) accept and reassemble correctly.

### 3. Permission / grant resolution conformance

The most error-prone area. Per [`storage-format.md`](storage-format.md) "Privacy and access control":

- **Resolution algorithm fixtures**: 30+ scenarios covering the precedence ladder (sensitivity-deny > channel-deny > type-deny > sensitivity-allow > channel-allow > type-allow > default_action). Each fixture: a defined dataset, a defined grant, a query → expected (events_returned, events_filtered, channels_stripped, audit_rows_emitted).
- **Operation-level scope**: `aggregation_only=1` blocks the right RPCs and allows the right ones; `strip_notes=1` strips only the right field; `require_approval_per_query=1` interacts correctly with notification triggers.
- **Time-window semantics**: `rolling_window_days=N` correctly excludes events at `now - N*86400_001 ms`; `grant_time_windows.from_ms` / `to_ms` boundaries exclusive vs. inclusive (spec'd as inclusive, fixture verifies).
- **Write scope**: `grant_write_event_type_rules` correctly accepts in-scope and rejects out-of-scope writes; `approval_mode='auto_for_event_types'` routes correctly per `grant_auto_approve_event_types`.
- **Pending event flow**: submitted → `pending` → approved → `events` row with same ULID. Audit rows for both submission and approval. Same flow with rejection.
- **Backdating**: a grantee's write with `timestamp_ms` outside `rolling_window_days` is rejected.
- **Case-bound grants**: case-bound grant scopes to events linked via `event_case_links` (per parallel emergency design); cases that are closed reject reads with `CASE_CLOSED`.

### 4. Sync conformance

Per [`sync-protocol.md`](sync-protocol.md):

- **Bidirectional replay**: cache and primary each start with disjoint event sets; one round of sync produces matching state in both. Watermarks advance as expected. Re-running sync is a no-op.
- **Idempotency under retry**: drop a `PushFrame` mid-stream; client retries; recipient inserts each at most once.
- **Tombstone propagation**: soft-delete on primary propagates to cache; cache no longer renders the event.
- **Correction propagation**: a `superseded_by` correction syncs and resolves correctly.
- **Pre-1970 events sync**: events with `timestamp_ms < 0` and clamped ULIDs sync without hash collisions or watermark errors.
- **Attachment lazy sync**: cache pulls attachment blobs on demand, not eagerly. SHA-256 verified on receipt.
- **Grant out-of-band**: cache calling `CreateGrantOnPrimary` → primary commits → next pull stream carries the new `Grant` row. Cache cannot create grants locally (returns `WRONG_TOKEN_KIND`).
- **Registry version skew**: cache on registry v3, primary on v5; primary advertises v5 in HelloResponse; cache successfully fetches the missing entries; subsequent sync proceeds.

### 5. Auth conformance

Per [`auth.md`](auth.md):

- **OAuth flows**: Authorization Code + PKCE (browser/MCP), Device Authorization Grant (CLI) — each produces a session token of the right shape (`ohds_…`) with correct TTLs.
- **OIDC verification**: id_token signature verification against a known JWKS; iss/aud/exp/nonce checks; rejection of expired / wrong-iss / wrong-aud tokens.
- **Refresh rotation**: every refresh issues a new refresh token; old token rejected on second use; replay of an already-rotated refresh marks all sessions as `compromise`.
- **Multi-identity linking**: `LinkIdentity` against a second provider correctly binds to the existing `user_ulid`; unlinking the last identity returns `LAST_IDENTITY`.
- **Account-join modes**: `open` / `invite_only` / `closed` all enforce correctly; invite redemption is single-use; email-bound invites verify the OIDC email matches.
- **Discovery**: `.well-known/oauth-authorization-server` returns valid RFC 8414 metadata; `.well-known/openid-configuration` mirrors.

## What the corpus does NOT cover

- **Performance / load** — separate benchmark suite. Conformance is correctness, not throughput.
- **Operator-side admin UX** — out of the OHDC protocol surface.
- **MCP server tool catalogs** — conventions, not protocol.
- **Care app / Connect mobile UX** — implementation behavior, not protocol.
- **Relay-side operational concerns beyond the wire spec** — bandwidth metering, abuse detection.
- **End-to-end channel encryption** — deferred per [`encryption.md`](encryption.md); when speccable, gets its own conformance section.

## Corpus structure

In the `openhealth-data/ohd-protocol` repo:

```
conformance/
├── manifest.yaml                       # corpus version, list of test groups, prerequisites
├── ondisk/                             # category 1
│   ├── sample_blocks/
│   │   ├── encoding1/
│   │   │   ├── 001_simple/
│   │   │   │   ├── input.json          # source samples + codec params
│   │   │   │   ├── expected.bin        # exact expected encoded bytes
│   │   │   │   └── README.md           # what this fixture proves
│   │   │   ├── 002_dense/...
│   │   │   └── ...
│   │   └── encoding2/...
│   ├── ulids/...
│   ├── registry_resolution/...
│   └── encryption_roundtrip/...
├── ohdc/                                # category 2
│   ├── put_events/
│   ├── query_events/
│   ├── aggregate/
│   ├── pagination/
│   ├── encoding_equivalence/
│   ├── error_codes/
│   └── streaming/
├── permissions/                         # category 3
│   ├── precedence_ladder/
│   ├── aggregation_only/
│   ├── strip_notes/
│   ├── time_windows/
│   ├── write_scope/
│   ├── pending_events/
│   └── case_bound/
├── sync/                                # category 4
│   ├── bidirectional_replay/
│   ├── retry_idempotency/
│   ├── attachments/
│   └── registry_skew/
└── auth/                                # category 5
    ├── oauth_flows/
    ├── oidc_verification/
    ├── refresh_rotation/
    └── account_join_modes/
```

Each fixture is a self-contained directory with:

- `input.{json,yaml}` — the test input (events to write, grant config, RPC requests, etc.)
- `expected.{json,yaml,bin}` — what the implementation should produce / contain after running.
- `README.md` — one-paragraph explanation of what's being tested and why.

Some fixtures share input data via `manifest.yaml` references; the manifest declares dependencies (e.g. "this fixture requires the standard registry to be loaded").

## Running the corpus

A small reference test runner lives in `ohd-protocol/conformance/runner/`. Implementations bring their own runner adapter (a thin shim that converts fixture inputs to the implementation's API); the runner orchestrates and compares.

```
$ cd ohd-protocol
$ ohdc-conformance --target=http://localhost:8000 --token=$TOKEN
running 423 tests
ondisk/sample_blocks/encoding1/001_simple ............ ok
ondisk/sample_blocks/encoding1/002_dense ............. ok
...
permissions/precedence_ladder/sensitivity_deny_wins .. ok
sync/bidirectional_replay/basic ...................... FAIL
  expected events_returned=42, got 41
  expected event ULID 01HF... in result set; missing
auth/refresh_rotation/double_use_compromise .......... ok

422 passed; 1 failed
```

CI runs the corpus against every PR to `ohd-storage`. New implementations bootstrap by running the corpus against their endpoint and watching the failures shrink.

## Versioning

Corpus version mirrors OHDC version: `conformance/v1.0.0/`. Patch bumps add fixtures (more coverage) and may tighten existing assertions; never weaken. Minor bumps add fixtures for new RPCs in the same OHDC major. Major bump (OHDC v2) is a new corpus.

When a fixture changes meaningfully (an assertion is tightened), the change goes into the next patch version with a note in the manifest. Implementations targeting a fixed corpus version pin against it.

## Adding a fixture

When a bug is fixed in `ohd-storage` or a spec ambiguity is resolved, the resolution arrives as a new corpus fixture in the same PR. Process:

1. Write the failing test against a fresh fixture directory (`conformance/<category>/<fixture>/`).
2. Implement the fix in the reference impl.
3. Verify the fixture now passes.
4. Submit both the fixture and the fix in the same PR.

The growing corpus is the durable record of what OHDC v1 actually requires, beyond what the spec docs prose-spec.

## What "claiming v1 conformance" means

A library, server, or service claiming OHDC v1 conformance must:

1. Pass the corpus at the latest patch version of `v1.x` (currently TBD as v1 is pre-release).
2. Display a conformance badge in their README / docs that includes the corpus version they pass against.
3. Document any optional features (encoding 2 sample blocks, source signing, etc.) they don't implement; the corpus includes both required-for-conformance and optional fixtures.

Vendors that claim v1 without passing the corpus are misleading; downstream consumers should rely on the badge or a corpus-run report, not on marketing copy.

## Cross-references

- OHDC RPC surface being tested: [`ohdc-protocol.md`](ohdc-protocol.md)
- On-disk format being tested: [`storage-format.md`](storage-format.md)
- Permission / resolution model: [`storage-format.md`](storage-format.md), [`privacy-access.md`](privacy-access.md)
- Sync model: [`sync-protocol.md`](sync-protocol.md)
- Auth model: [`auth.md`](auth.md)
- Roadmap entry for the conformance corpus repo: [`../roadmap.md`](../roadmap.md) "Cross-cutting tasks"

## Open items

- **Initial fixture authorship.** This doc specifies the structure; the actual fixture content has to be written alongside the reference implementation. Phase 1 work for Task #16's continuation.
- **Performance benchmarks** as a separate suite (not conformance).
- **End-to-end channel encryption fixtures** when that mechanism is specced.
- **Cross-implementation runner harness** — currently each implementation brings its own adapter; a Connect-RPC-native runner (using the `.proto` directly) would be cleaner for the long-tail.
