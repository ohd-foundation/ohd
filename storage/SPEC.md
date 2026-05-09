# OHD Storage — Implementation-Ready Spec

> Working summary of what OHD Storage actually has to build, distilled from
> the design docs in [`spec/`](spec/). For the full canonical text see those
> docs; this file is the implementation phase's working notes.

## 1. Responsibilities

Storage is the data layer of OHD. It:

- Stores health events in a portable, documented on-disk format (one file per
  user; see [`spec/storage-format.md`](spec/storage-format.md)).
- Validates writes against the channel registry (`event_types`, `channels`,
  with append-only enum ordinals and canonical units).
- Resolves OHDC requests under one of three auth profiles
  (self-session / grant / device) — see §4.
- Enforces grant rules on reads and writes; routes grant-mediated writes
  through the approval queue when policy requires.
- Audits every external operation (accepted, partial, rejected, errored).
- Replicates between deployments (cache ↔ primary) via bidirectional
  event-log replay — see [`spec/sync-protocol.md`](spec/sync-protocol.md).
- Encrypts the file at rest with a per-user key (SQLCipher 4 + libsodium for
  sidecar blobs) — see [`spec/encryption.md`](spec/encryption.md).

What storage does **not** do:

- It does not host a UI (Connect, Care, Emergency do).
- It does not collect data on its own (consumers do).
- It does not analyze data (consumers do).
- It does not expose multiple external APIs — OHDC is its only external
  surface, and what an authenticated session can do is determined by token
  scope, not protocol layer.

## 2. On-disk schema (summary)

Full schema in [`spec/storage-format.md`](spec/storage-format.md) "SQL
schema". Key tables:

| Table | Role |
|---|---|
| `_meta` | format version, `user_ulid`, `deployment_mode`, KDF params, audit retention, emergency-feature toggles |
| `event_types`, `channels`, `type_aliases`, `channel_aliases` | registry — tree-structured per event type, append-only enum ordinals, canonical units |
| `events` | one row per measurement act; `ulid_random` (10 bytes) + `timestamp_ms` (signed Unix ms) form the wire ULID |
| `event_channels` | EAV — sparse per-channel scalar values |
| `event_samples` | dense numeric streams as compressed sample blocks (codecs 1, 2) |
| `attachments` | metadata; payloads live in sidecar `blobs/` |
| `devices`, `app_versions` | provenance |
| `grants` + rule tables (`grant_event_type_rules`, `grant_channel_rules`, `grant_sensitivity_rules`, `grant_write_event_type_rules`, `grant_auto_approve_event_types`, `grant_time_windows`, `grant_cases`) | access control |
| `pending_events` | grant-submitted writes awaiting user approval |
| `cases`, `case_filters`, `case_reopen_tokens` | curated containers for episodes (visits, admissions, emergencies, cycles) |
| `audit_log` | append-only; one row per RPC, including rejections |
| `peer_sync` | replication watermarks per known peer |

**Key invariants:**

- Events are immutable. Corrections are new events with `superseded_by`.
- Soft delete via `deleted_at_ms`; rows are not physically removed.
- ULID's time prefix equals `timestamp_ms` for events with `timestamp_ms >= 0`;
  clamped to `0` for pre-1970 events. The 80-bit random half is unique on
  its own (covered by `idx_events_ulid`).
- `(source, source_id)` enforces idempotency on writes (see
  `idx_events_dedup`).
- Privacy is structural: `event_type.default_sensitivity_class` and
  `channel.sensitivity_class` are the only privacy carriers — no per-event
  tags.
- Grants don't chain. Only self-session callers can create / update / revoke
  grants.

## 3. OHDC operations storage implements

Full wire spec in [`spec/ohdc-protocol.md`](spec/ohdc-protocol.md). Three
services plus HTTP-only OAuth/discovery endpoints.

### `OhdcService` (consumer surface)

- **Writes**: `PutEvents`, `AttachBlob` (client-streaming).
- **Reads**: `QueryEvents` (server-streaming), `GetEventByUlid`, `Aggregate`,
  `Correlate`, `ReadSamples` (server-streaming), `ReadAttachment`
  (server-streaming).
- **Grants**: `CreateGrant`, `ListGrants`, `UpdateGrant`, `RevokeGrant`.
  All self-session-only; `RevokeGrant` is synchronous against the primary.
- **Cases**: `CreateCase`, `UpdateCase`, `CloseCase`, `ReopenCase`,
  `ListCases`, `GetCase`, `AddCaseFilter`, `RemoveCaseFilter`,
  `ListCaseFilters`.
- **Audit**: `AuditQuery` (server-streaming, optional tail mode).
- **Pending events**: `ListPending`, `ApprovePending`, `RejectPending`.
- **Export / Import**: `Export` (server-streaming, signed by source
  instance), `Import` (client-streaming, self-session-only).
- **Diagnostics**: `WhoAmI`, `Health` (only unauthenticated RPC).

### `AuthService`

Identity, sessions, device-token issuance, push-token registration. See
[`spec/`](spec/)'s `auth.md` (in the global spec) for the full design;
storage implements the server side.

### `SyncService` (intra-storage)

Used between a user's own instances. `Hello`, `PushFrames`,
`PullFrames`, attachment-blob transfer, plus `CreateGrantOnPrimary` /
`RevokeGrantOnPrimary` / `UpdateGrantOnPrimary` (RPC-gated, not stream-
replicated).

### `RelayService` (storage ↔ relay)

`Register`, `RefreshRegistration`, `Heartbeat`, `Deregister`, `OpenTunnel`
(long-lived bidi stream of opaque ciphertext frames).

### HTTP-only endpoints

`/.well-known/oauth-authorization-server`,
`/.well-known/openid-configuration`, `/authorize`, `/oidc-callback`,
`/token`, `/device`, `/oauth/register`, `/health`, `/metrics`.

## 4. Auth profile resolution

Token prefix → kind:

- `ohds_…` → **self-session**
- `ohdg_…` → **grant**
- `ohdd_…` → **device**

Resolution algorithm on every request:

1. Read `Authorization: Bearer <token>`. Missing / malformed →
   `UNAUTHENTICATED`.
2. Classify by prefix. Unknown prefix → `UNAUTHENTICATED`.
3. Self-session: validate against the OIDC session table; map subject →
   `_meta.user_ulid`. Fail closed.
4. Grant / device: look up the grant row by `ulid_random`; check
   `expires_at_ms`, `revoked_at_ms` → `TOKEN_EXPIRED` / `TOKEN_REVOKED`.
5. Confirm the operation is allowed for this kind (token-kind matrix in
   `spec/ohdc-protocol.md`); on mismatch → `WRONG_TOKEN_KIND`.
6. For grant tokens: check operation-level scope (`aggregation_only`,
   `require_approval_per_query`, rate limits); intersect filter with grant
   rules.
7. For device tokens: confirm op is write-only and event types are within
   the device's allowlist.
8. Run the operation. Append exactly one `audit_log` row regardless of
   outcome.

## 5. Write-with-approval

Per-grant policy field `approval_mode`:

- `always` — every submission queues into `pending_events`.
- `auto_for_event_types` — submissions whose event type is in
  `grant_auto_approve_event_types` commit immediately; others queue.
- `never_required` — all submissions commit immediately. Used for trusted
  long-term grants and emergency cases.

Pending-event flow:

1. `PutEvents` from a grant token; storage validates against the registry
   and authorizes against `grant_write_event_type_rules`.
2. Decide commit-or-queue per `approval_mode`. Allocate the ULID either way.
3. On queue: insert into `pending_events` (`status='pending'`, `payload_json`
   = canonical Protobuf-JSON of the original `Event`); notify the user.
4. User reviews via OHD Connect (`ApprovePending` / `RejectPending`).
5. Approve: insert into `events` with the same ULID; set
   `pending_events.status='approved'` and `approved_event_id`. Audit both
   submission and approval.
6. Reject: set `status='rejected'` and optional reason. Audit both.
7. Auto-expire after `expires_at_ms` → `status='expired'`.

ULID identity is preserved across pending → committed.

## 6. Sync model

Full spec in [`spec/sync-protocol.md`](spec/sync-protocol.md).

- Per-file `_meta.deployment_mode` ∈ {`primary`, `cache`, `mirror`}.
- Cache talks to primary via `SyncService` with the user's self-session
  token (no special peer credential).
- Per-peer rowid watermarks in `peer_sync`. Two are tracked:
  `last_outbound_rowid` (highest local rowid the peer has acked) and
  `last_inbound_peer_rowid` (highest peer rowid we have consumed).
- Steady-state cycle: `Hello` → `PushFrames` (outbound) ↔ `PullFrames`
  (inbound) → attachment payload transfer for any new attachment ULIDs.
- Dedup on `ulid_random`; set `origin_peer_id` on imported rows.
- **Grants do NOT stream-replicate.** Cache calls
  `CreateGrantOnPrimary` / `RevokeGrantOnPrimary` /
  `UpdateGrantOnPrimary`; the result row replicates inbound on the next
  pull pass. This makes "I revoked my doctor's access just now" act with
  the right semantics.
- **Audit log doesn't sync** — each instance audits its own access; remote
  imports are audited as `actor_type='system'`, `action='import'`.
- **Pending events DO sync** — so the user sees them on every device.
- Tombstones (soft deletes) propagate as ordinary rows.

## 7. Encryption at rest

Full spec in [`spec/encryption.md`](spec/encryption.md). Three keys per user:

- **`K_file`** — SQLCipher 4 page-encryption key (and libsodium key for
  sidecar blobs). 256 bits. Long-lived, rotates on demand. Held in process
  memory (`zeroize::Zeroizing`); zeroed on lock / process exit.
- **`K_envelope`** — encrypts `K_file` for storage. Re-derived on every
  unlock from a user secret + per-file salt. KDF: PBKDF2-SHA512 today
  (SQLCipher 4 default, 256k iterations); Argon2id when SQLCipher 5 lands.
- **`K_recovery`** — derived from a 24-word BIP39 phrase shown once at
  first launch.

Per-deployment-mode key flow (on-device vs. cloud) covered in detail in
the design doc. Cloud topology: Connect derives `K_envelope` locally,
wraps `K_file` locally, uploads only the wrap; transmits `K_file` to the
server inside the OHDC TLS session per-unlock (`Auth.UnlockFile`); server
zeroes on session end.

## 8. Deployment topologies

| Topology | Operator | Reachable directly? |
|---|---|---|
| On the user's phone | The user (their device) | No — needs OHD Relay for external access |
| OHD Cloud | The OHD project | Yes |
| Custom provider (clinic / insurer / ...) | The third party | Yes |
| Self-hosted (VPS / NAS / home server) | The user | Maybe — needs OHD Relay if behind NAT |

Same code, same on-disk format, in all four topologies. Linux server
deployments are fronted by Caddy (auto-HTTPS, HTTP/3). Mobile deployments
link the storage core in-process via uniffi.

Operator-side concerns (Docker Compose, Hetzner, backups, monitoring) live
in `../spec/docs/design/deployment.md` — outside this component's
responsibility, but storage's binary is the workload they target.

## 9. Conformance requirements

Full corpus in [`spec/conformance.md`](spec/conformance.md). To claim OHDC v0
conformance, an implementation must:

1. Compile and serve the canonical `.proto` files in `proto/ohdc/v0/`
   unchanged (no field deletion, no renumbering, no semantic drift).
2. Pass the conformance corpus end-to-end:
   - **On-disk format**: schema migration round-trips, sample-block byte
     equality across both encodings, ULID generation determinism (including
     pre-1970 clamping), channel-tree resolution.
   - **OHDC RPC**: happy-path + every error code from the catalog;
     `application/proto` ↔ `application/json` encoding equivalence;
     idempotency via `(source, source_id)`; pagination order; filter
     language predicates; streaming framing.
   - **Permission / grant resolution**: 30+ scenarios covering the precedence
     ladder (sensitivity-deny > channel-deny > type-deny > sensitivity-allow
     > channel-allow > type-allow > default_action); operation-level scope;
     time-window semantics; write-scope; pending-event flow; backdating.
   - **Sync**: bidirectional replay convergence; idempotency under retry;
     tombstone + correction propagation; pre-1970 events syncing; attachment
     lazy sync; grant out-of-band.
   - **Auth**: OAuth flows; OIDC verification; refresh rotation;
     multi-identity linking; account-join modes; discovery metadata.
3. Honor every error code in `spec/ohdc-protocol.md` with the correct HTTP
   status mapping.
4. Implement every RPC in the token-kind matrix with the correct scope
   behavior.
5. Expose the HTTP-only endpoints at canonical paths.

Vendors may add custom RPCs to `com.<vendor>.ohdc_ext.v0.*` namespaces; those
are out of scope for v0 conformance and don't claim it.

## 10. Open items (forwarded to implementation phase)

- **End-to-end channel encryption** for the most sensitive sensitivity
  classes (`mental_health`, `sexual_health`, etc.). The data model leaves
  room; the key-wrapping and grant-side ciphertext semantics are not yet
  spec'd.
- **Standard registry governance** — how new entries get added (PR + version
  bump, registry council, etc.). Deferred until the project has more than
  one contributor.
- **Family / delegate access** — grants with `grantee_kind='delegate'`. Full
  vs. scoped authority TBD.
- **Sync wire frame catalog completeness** — `RegistryEntryFrame.payload`
  needs a tagged-union shape per entry kind.
- **Relay TLS-through-tunnel cert/identity model** — the `OpenTunnel` frame
  shape and storage's self-signed cert model are open.
- **Operator-side admin RPCs** — invite-management beyond what users issue;
  deployment configuration; tenant management. Belongs in a separate
  `OperatorService` later; not v1.
- **`WatchEvents` subscription RPC** — additive in a v1.x if Connect web
  needs live charts.
