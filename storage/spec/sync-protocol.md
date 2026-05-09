# Design: Cache ↔ Primary Sync Protocol

> The wire-level spec for replicating events between a `cache` instance (typically a phone-side mirror) and a `primary` instance (typically a remote server). Layered on top of OHDC + a small set of sync-specific RPCs.
>
> Pairs with [`storage-format.md`](storage-format.md) (which fixes the *logical* sync model — bidirectional event-log replay, ULID-based dedup, per-peer rowid watermarks, `origin_peer_id` to avoid echo). This doc is the wire shape: how those primitives flow over the network.

## What sync is and isn't

**Sync is for**: a user with multiple OHD instances of *their own* data (e.g. phone-cache + cloud-primary, or cloud-primary + NAS-mirror) keeping them eventually consistent. One canonical authority (the primary) accepts all external writes and grant queries; other instances mirror, and may queue local writes that flush to the primary on next contact.

**Sync is not**: cross-user replication. The protocol replicates one user's data between their own instances. Cross-user shapes (a doctor's grant pulling data into Care, a researcher's cohort study) are OHDC reads, not sync.

## Roles

Per [`storage-format.md`](storage-format.md) "Deployment modes and sync":

- **`primary`** — canonical for the user. Accepts external writes (sensor pushes, grant submissions, user writes), serves grant queries, runs the full grant resolution algorithm.
- **`cache`** — mirrors a remote primary. Local writes go into a local-write queue and flush to the primary; remote-origin events sync inbound. Cannot serve external grant queries (those route to the primary).
- **`mirror`** — read-only replica. Backups, hot standbys. Receives but never writes.

Sync moves events between these roles. The roles are set at file creation (`_meta.deployment_mode`) and don't change without an explicit migration.

## What gets synced

- **Events** (the `events` row + its child `event_channels` + `event_samples`)
- **Attachments** (metadata + sidecar blob payloads)
- **Pending events** (so the user sees "Dr. Smith submitted a write" on every device, not just the primary)
- **Grants** (all grant rules and policy fields; needed for offline-cache to know what was/is allowed)
- **Devices** and **app_versions** (the registry rows event provenance points at)
- **Standard registry alias entries** (`type_aliases`, `channel_aliases`) — needed for deterministic lookup across instances
- **Custom registry entries** (`com.<owner>.*` event types and channels)

## What doesn't get synced

- **Audit log entries** — each instance audits its own access. When a remote-origin event is imported, an audit row is written with `actor_type='system'`, `action='import'`. Audit doesn't propagate; it's per-eyeball.
- **Sessions / OAuth tokens** — system DB; per-instance.
- **OIDC identities** — system DB; per-instance.
- **Push tokens / notification preferences** — per-device.
- **Storage-relay registrations** — per-instance.
- **System DB rows in general** — only the per-user file syncs.

## The sync RPC surface

Sync is **its own service** in the OHDC `.proto` package, separate from `OhdcService` (which is consumer-facing) and `RelayService` (which is relay-facing). Used only between the user's own instances.

```protobuf
service SyncService {
  // Discovery: peer says "hello", advertises capabilities and known watermarks.
  rpc Hello(HelloRequest) returns (HelloResponse);

  // Outbound batch: the calling instance pushes events the peer hasn't seen yet.
  // Returns acks for each frame.
  rpc PushFrames(stream PushFrame) returns (stream PushAck);

  // Inbound batch: the calling instance asks the peer for events it doesn't have.
  // Server-streaming.
  rpc PullFrames(PullRequest) returns (stream PushFrame);

  // Attachment payload transfer (separate channel from event metadata for chunking).
  rpc PushAttachmentBlob(stream AttachmentChunk) returns (AttachmentAck);
  rpc PullAttachmentBlob(PullAttachmentRequest) returns (stream AttachmentChunk);

  // Grant lifecycle is RPC-gated, NOT stream-replicated.
  // (Synchronous against the primary per privacy-access.md; this RPC lets a cache
  // call CreateGrant / RevokeGrant / UpdateGrant against the primary.)
  rpc CreateGrantOnPrimary(CreateGrantRequest) returns (CreateGrantResponse);
  rpc RevokeGrantOnPrimary(RevokeGrantRequest) returns (RevokeGrantResponse);
  rpc UpdateGrantOnPrimary(UpdateGrantRequest) returns (Grant);
}
```

Where `CreateGrantRequest` / `Grant` / etc. are the same messages from [`ohdc-protocol.md`](ohdc-protocol.md). Sync just gives the cache a way to invoke them on the primary while presenting *the user's self-session token* (cache → primary auth is via the user's same OIDC identity).

## Auth between peers

A cache talking to its primary uses the **user's self-session token** — the same `ohds_…` the user holds. The primary treats the cache like any other self-session client (it is one, behaviorally). Sync doesn't need a special auth profile; the user is authenticating themselves to their own data.

Any peer instance the user pairs (cloud primary + new phone cache, etc.) goes through standard OAuth flow against the primary, gets a self-session token, then runs `SyncService` calls. No magic peer credentials.

## The sync flow (steady state)

A cache running `SyncService` against its primary, every N seconds (default 60s when foregrounded, push-driven when backgrounded):

```
1. cache → primary: Hello(my_local_rowid_high_water=X, my_inbound_watermark_for_you=Y)
   primary → cache: HelloResponse(my_local_rowid_high_water=A, my_inbound_watermark_for_you=B,
                                  registry_version=N, peer_id=P)

2. Outbound (cache pushes its local writes to primary):
   if X > primary's last-acked-from-cache (= cache's peer_sync.last_outbound_rowid):
     cache → primary: PushFrames(stream of events with id > last_outbound_rowid AND origin_peer_id IS NULL)
     primary → cache: PushAcks(per frame; updates last_outbound_rowid as each succeeds)

3. Inbound (cache pulls from primary):
   cache → primary: PullFrames(after_peer_rowid=cache's peer_sync.last_inbound_peer_rowid)
   primary → cache: stream of PushFrame(events with primary's local rowid > after_peer_rowid)
                    Each frame carries primary's rowid as the watermark advancer.
   cache: insert each, dedupe on ulid_random, set origin_peer_id = primary's peer_id, advance watermark.

4. Attachment payloads:
   For any new attachment ULIDs from steps 2/3, separately PushAttachmentBlob / PullAttachmentBlob
   for the bytes. Idempotent on sha256.
```

Steps 2 and 3 run independently (no required ordering). Steps run in parallel where the implementation can.

## Frames

A `PushFrame` carries one logical entity:

```protobuf
message PushFrame {
  // Sender's local rowid for this entity. Recipient uses this to advance watermark.
  int64 sender_rowid = 1;

  oneof entity {
    EventFrame event = 2;
    PendingEventFrame pending_event = 3;
    GrantFrame grant = 4;             // for replicating grant rows AFTER they were created on primary
    DeviceFrame device = 5;
    AppVersionFrame app_version = 6;
    AliasFrame alias = 7;             // type or channel alias
    RegistryEntryFrame registry_entry = 8;  // custom event_types / channels
    DeleteFrame deleted = 9;          // soft delete (sets deleted_at_ms)
    SupersedeFrame supersede = 10;    // a `correction` event linkage update
  }
}

message EventFrame {
  // Full Event payload (same shape as ohdc-protocol.md Event message)
  Event event = 1;
}

message PendingEventFrame {
  PendingEvent pending = 1;
}

message GrantFrame {
  Grant grant = 1;
}

message DeleteFrame {
  Ulid event_ulid = 1;
  int64 deleted_at_ms = 2;
}

// ... DeviceFrame, AppVersionFrame, AliasFrame, RegistryEntryFrame, SupersedeFrame
//     each carry the row content needed to recreate / update on the recipient.
```

`PushAck` per frame:

```protobuf
message PushAck {
  int64 sender_rowid = 1;             // echoes the frame's rowid (so sender can advance)
  oneof outcome {
    PushAccepted accepted = 2;
    PushDuplicate duplicate = 3;      // already had this ULID; sender advances anyway
    PushError error = 4;
  }
}
```

`PullFrames(after_peer_rowid)` returns a stream of `PushFrame`s with the same shape; recipient just inserts.

## Idempotency and deduplication

Per [`storage-format.md`](storage-format.md) "Sync protocol":

- **ULID identity** is the dedup key. Insert events with `INSERT OR IGNORE`-style behavior; if the ULID already exists, the frame is silently a no-op (returns `PushDuplicate`).
- **Watermarks** are based on **insertion order** (rowid), not on event timestamp or ULID time. Backfilled events (and pre-1970 events with clamped ULID time prefix) sync correctly because rowid is monotonic by insertion regardless of timestamp.
- **`origin_peer_id`** prevents echoes. A cache that received an event from primary marks `origin_peer_id` to the primary's peer_id; cache's outbound push only sends events with `origin_peer_id IS NULL` (i.e., locally-minted, not echoed back).
- **Corrections, soft-deletes, and grant rules** are ordinary rows that ride the same dedup machinery. No conflict resolution; immutability + ULID + tombstones do the work.

## Conflicts

There are essentially none, by design:

- **Two events with the same ULID**: structurally impossible (CSPRNG collision-safe).
- **Two devices write the same idempotency `(source, source_id)` pair**: the second write fails with `IDEMPOTENCY_CONFLICT` at the sending side before sync ever sees it.
- **A correction (`superseded_by`) on an event that doesn't exist on the recipient yet**: recipient queues the correction, applies when the underlying event arrives. Or if it never arrives (data loss), correction sits as orphan; cleanup tooling can detect.
- **A delete on an event with corrections referencing it**: corrections still resolve (they reference the original ULID, not the row); the original is soft-deleted, view layer renders accordingly.

## Grant lifecycle: out-of-band, not stream-replicated

Per [`privacy-access.md`](privacy-access.md) "Revocation semantics" and [`storage-format.md`](storage-format.md):

- **Create grant** from a cache: `SyncService.CreateGrantOnPrimary(req)` — cache forwards the user's intent to the primary, primary creates the grant, returns the token. Cache does NOT create grants locally.
- **Revoke grant** from a cache: `SyncService.RevokeGrantOnPrimary(req)` — synchronous, fail-loud. The user's expectation is "I revoked, the doctor can't read anymore" — only correct if revocation is immediate at the primary.
- **Update grant** similarly.

After the primary commits the grant change, the next regular sync pull from cache will receive the updated `Grant` row as an ordinary `GrantFrame` and update the cache's local mirror. The cache uses the local mirror for *display* (showing the user "you have these grants"); it does not enforce grant scope (only the primary does).

This is the asymmetry: events stream-sync, grants RPC-sync. Same `SyncService` channel, different operation kind.

## Attachment payload sync

Attachment metadata (the `attachments` row + `AttachmentRef` on events) syncs in the normal frame stream. The sidecar **blob payload** does not — it's pulled separately on demand:

```
cache: notices an EventFrame references attachment ULID X with sha256 H
cache: checks if blobs/<H> exists locally
       if yes: nothing to do
       if no: SyncService.PullAttachmentBlob(attachment_ulid=X) → stream of AttachmentChunk
              cache: writes blobs/<H>, encrypts with the per-user key per encryption.md
```

Bidirectional: a cache that locally created an attachment (user took a photo, attached a PDF) pushes the blob via `PushAttachmentBlob` after the metadata frame is acked.

Blob sync is **lazy**: caches only fetch blobs they need (user opens an event with an attachment → fetch then). Avoids syncing 100 GB of historical attachment payloads to a phone that will rarely look at them. Storage pressure is the user's choice; the user can request a "full mirror" mode (download all blobs eagerly) per attachment-type or globally.

## Failure handling

- **Network drop mid-stream**: client retries with the same watermark; server resumes naturally (idempotent ULID dedup absorbs any redelivered frames).
- **Server returns error mid-frame**: client ack-up-to-the-error-point, retries the failed frame. After 5 consecutive failures on the same frame (e.g. a registry-entry that the recipient's older spec version doesn't recognize), client logs the failure and skips with a warning (continues syncing the rest).
- **Watermark divergence** (cache's `last_outbound_rowid` higher than primary's view): can't normally happen, but defensive handling — primary tells cache the lower bound; cache resyncs from that.
- **Registry version mismatch**: if primary is on registry v5 and cache is on v3, primary advertises `registry_version=5` in HelloResponse; cache fetches the missing standard registry entries via `PullFrames` filter for `RegistryEntryFrame`s, applies, then proceeds with normal sync. If cache can't accept (older client doesn't know what to do with new types), it falls back to syncing only types it knows; new-type events arrive but get stored as opaque `Event` rows the cache can't render (data preserved, no display).

## Sync orchestration

When does sync run?

- **Cache foregrounded** (Connect mobile in active use): periodic (60s) + on local write (push immediately).
- **Cache backgrounded** (phone in pocket): less aggressive — every 30 min via OS background work; on push-wake from primary if primary has events for cache.
- **Primary** (always-on server): doesn't proactively run sync; it responds to cache-initiated calls. Optionally pings caches via push when high-priority events arrive (a pending event the user should review).
- **Initial sync** (new cache pairing): full pull from primary, can take minutes for a large file. Progress UI in Connect.

Throttling: heavy sync paths can be rate-limited (`max_sync_bandwidth_bytes_per_second` deployment setting on primary) for shared infrastructure where a single user's catch-up sync shouldn't dominate.

## Auth + transport recap

- Sync uses Connect-RPC over HTTP/3 (HTTP/2 fallback), same transport as OHDC consumer traffic.
- Self-session token in `Authorization: Bearer ohds_…`.
- TLS 1.3.
- For relay-mediated primaries (cache talking to a phone-hosted primary, or vice versa): the same tunnel + cert pin mechanism from [`relay-protocol.md`](relay-protocol.md) applies — sync RPCs ride on inner TLS through the relay tunnel.

## What isn't here in v1

- **Multi-primary** topologies (two equal primaries the user uses interchangeably, with conflict resolution). v1 is single-primary; one canonical writer at a time.
- **Selective sync filters** ("only sync events from the last 6 months to my phone"). Future revision; for now caches sync everything.
- **Snapshot-and-rebase** for caches that fall far behind (currently they catch up event by event). Could matter for users who haven't used a phone in a year. Optional optimization.
- **Bandwidth-adaptive batching** at the wire level. Implementations may batch frames opportunistically; the spec doesn't mandate.

## Implementation effort

The sync layer is ~800 lines of Rust on top of the existing OHDC server: SyncService implementation, watermark management, frame serialization, retry logic. Most of the complexity is already absorbed by `storage-format.md`'s primitives (immutability, ULID dedup, rowid watermarks, origin_peer_id). The wire spec just makes those primitives addressable over the network.

## Cross-references

- Logical sync model (immutability, ULID dedup, watermarks, origin_peer_id): [`storage-format.md`](storage-format.md) "Deployment modes and sync"
- OHDC RPC reference (CreateGrant, etc., used by SyncService.*OnPrimary): [`ohdc-protocol.md`](ohdc-protocol.md)
- Revocation semantics (why grants don't stream-sync): [`privacy-access.md`](privacy-access.md)
- Tunnel transport for relay-mediated primaries: [`relay-protocol.md`](relay-protocol.md)
- Encryption (per-user key consistent across cache and primary; identity key separate): [`encryption.md`](encryption.md)

## Open items (forwarded)

- **Multi-primary support** — explicit conflict resolution, vector clocks or per-channel last-writer-wins. Not v1.
- **Selective sync** — per-event-type / per-time-range exclusions on caches. Not v1.
- **Sync over OHD Relay TURN-style for cache↔cache** (two phones syncing without a primary). Probably never; primary-mediated is simpler.
- **Sync-driven garbage collection for stale tombstones** — when primary purges audit / event soft-deletes per `audit_retention_days`, cache should follow. Mostly works naturally because the rows just don't sync; explicit tombstone-of-tombstone may be needed.
