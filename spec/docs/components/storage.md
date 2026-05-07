# Component: OHD Storage

> The data layer. Owns persistence, permissions, audit, grants, sync, and encryption. Exposes its functionality only through the **OHDC** protocol.

## Purpose

OHD Storage is the source of truth for a person's health data. It:

- Stores events in a documented, portable on-disk format (see [`../design/storage-format.md`](../design/storage-format.md)).
- Validates writes against the channel registry.
- Resolves OHDC requests under the appropriate auth profile (self-session / grant / device).
- Enforces grant rules on reads and writes (with optional approval queue for grant-mediated writes).
- Audits every access.
- Replicates between deployments (cache ↔ primary) using bidirectional event-log replay.
- Encrypts data at rest with a per-user key.

What OHD Storage does **not** do:

- It does not host a UI. UIs are OHD Connect (personal) or OHD Care (professional) consumers.
- It does not collect data on its own. Data collection is via OHDC consumers.
- It does not analyze data. Analysis lives in consumers (apps, CLIs, MCP servers).
- It does not expose multiple external APIs. Its only external surface is the OHDC protocol; what an authenticated session can do is determined by token scope, not protocol layer.

## Deployment topologies

OHD Storage runs in any of four topologies (the user chooses at app setup); all four use the same code and same on-disk format. See [`../deployment-modes.md`](../deployment-modes.md) for the user-facing tradeoffs.

| Topology | Operator | Reachable directly? |
|---|---|---|
| On the phone | The user (their device) | No — needs OHD Relay for remote access |
| OHD Cloud | The OHD project | Yes |
| Custom provider (clinic / insurer / employer / etc.) | The third party | Yes |
| Self-hosted (VPS / NAS / home server) | The user | Maybe — needs OHD Relay if behind NAT without port forwarding |

See [`../design/deployment.md`](../design/deployment.md) for operator-side details (Docker Compose, backups, Caddy, Hetzner provisioning) and [`relay.md`](relay.md) for the bridging service that handles unreachable storage.

## External interface — OHDC

OHD Storage exposes a single external protocol: **OHDC**. Three auth profiles flow through it, each enforced server-side:

| Auth profile | Used by | Scope |
|---|---|---|
| **Self-session** | The user themselves (OHD Connect personal app, personal CLI, personal MCP) | Full scope on own data: read, write, manage grants, view audit, export |
| **Grant token** | Third parties (OHD Care, researcher portals, family / delegate access) | Bounded by the grant's structured rules — read scope, write scope, approval policy, time windows, rate limits, expiry |
| **Device token** | Sensor / lab / pharmacy / EHR integrations and the user's own per-device write services | Write-only, no expiry, attributed by device |

Different auth bars for different damage profiles. Device tokens are cheap to grant (Libre's CGM service gets one per user, bounded to write events attributed as Libre device); grant tokens require explicit, expiring user issuance with structured scope; self-session tokens require full OIDC.

A single API surface plus auth-driven scope makes mixed cases natural — e.g. a doctor with a grant that allows read of vitals plus write-with-approval of `lab_result` and `clinical_note` is one grant, not two protocols.

See [`connect.md`](connect.md) for the OHDC protocol spec and the personal-side reference consumer.

## What lives in storage

The file format is the spec; this component runs against it. See [`../design/storage-format.md`](../design/storage-format.md) for the schema. At a glance:

- `events`, `event_channels`, `event_samples` — the data
- `event_types`, `channels`, `type_aliases`, `channel_aliases` — the registry
- `devices`, `app_versions` — provenance
- `grants`, plus rule tables (`grant_event_type_rules`, `grant_channel_rules`, `grant_sensitivity_rules`, `grant_time_windows`, `grant_write_event_type_rules`) — access control
- `pending_events` — events submitted under a grant with `require_user_approval_for_writes`, awaiting user review
- `audit_log` — every access
- `peer_sync` — replication watermarks
- `_meta` — file format version, deployment mode, user ULID, etc.
- `attachments` — references to sidecar blob files

## Write-with-approval

Grant tokens with write scope can be configured (per the grant's `approval_mode`) to route submissions through an approval queue. Submitted events land in `pending_events` rather than `events`; the user reviews them in OHD Connect; on approval the event commits to `events` with the same ULID. See `../design/storage-format.md` "Privacy and access control / Write-with-approval" for the full mechanism.

This makes grant-mediated writes safe: a doctor can submit lab results, observations, clinical notes; the patient retains per-event control over what enters their canonical record. Trust-tiered policy (`always` / `auto_for_event_types` / `never_required`) tunes the experience for new vs. established relationships.

## Sync between deployments

When a user runs both a cache (e.g. phone) and a primary (e.g. SaaS), the two storages exchange events via bidirectional event-log replay. ULID identity makes inserts idempotent; per-peer rowid watermarks make sync robust to backfill and pre-1970 events. Corrections, soft deletes, and grant rows all sync as ordinary rows.

Grant *revocations* and *creates* do **not** sync — they're synchronous RPCs against the primary, not part of the replication stream. This makes "I revoked my doctor's access just now" act with the right semantics: either succeed immediately and propagate, or fail loudly to the user.

See `../design/storage-format.md`, sections "Deployment modes and sync" and "Privacy and access control / Revocation semantics."

## Bridging via OHD Relay

When the storage is not directly reachable from the public internet (on-device, or self-hosted behind NAT without port forwarding), OHD Relay forwards opaque packets between storage and remote OHDC clients. Relay is a separate deployable component, not part of OHD Storage. See [`relay.md`](relay.md).

## Implementation

Single Rust core implementing the on-disk format and the OHDC interface, distributed as:

- A Linux binary for OHD Cloud / custom-provider / self-hosted deployments, fronted by Caddy for HTTP/3.
- An `.aar` (with `.so` per ABI) for Android, linked into OHD Connect for Android.
- An `.xcframework` for iOS, linked into OHD Connect for iOS.
- A `PyO3`-bound Python wheel for server-side scripting and the conformance test harness.

Bindings are generated with `uniffi` (Kotlin, Swift) and `PyO3` (Python), so each platform sees idiomatic types without hand-written FFI shims. The on-disk format is byte-identical across platforms; a file written by Android can be opened by Linux unchanged, and vice versa.

The engine is SQLite + SQLCipher. Concurrency is single writer + many readers per file (WAL mode). Cross-process access to the same file is not supported — one process owns the writer.

A conformance corpus (a known input event sequence + expected query outputs) is part of the format spec, so any future re-implementation can be checked for byte and semantic compatibility.

## Operational concerns

For OHD Cloud / custom-provider / self-hosted deployments:

- **Backups**: nightly snapshots of the per-user files → encrypted object storage, with rotation. The portable export is also a user-facing escape hatch — always available, including in a read-only failure mode.
- **Monitoring**: structured JSON logs from the storage process; `/health` endpoint; Prometheus `/metrics`.
- **Authentication**: OIDC delegation for user identity. The storage tracks an opaque `user_ulid` mapped to an OIDC `(provider, subject)`; no PII in the storage protocol itself.
- **TLS**: Caddy front; automatic HTTPS via Let's Encrypt; HTTP/3 enabled by default.

For phone deployments these are mostly handled by the OS / app shell.

## Open design items

These are documented in [`../design/storage-format.md`](../design/storage-format.md) and not yet specified at the bit level:

- **End-to-end channel encryption** for the most sensitive sensitivity classes (mental_health, sexual_health, etc.). The data model leaves room for it; the key-wrapping and grant-side ciphertext semantics are not yet spec'd.
- **Standard registry governance**. The catalog ships with the format; how new entries get added (PR + version bump, registry council, etc.) is left for when the project has more than one contributor.
- **Family / delegate access**. A grant kind where one user acts on behalf of another. Modeled as `grants.kind='delegate'`; full or scoped authority TBD.
