# Glossary

> Every term, abbreviation, and namespace used in the OHD spec, defined once.

## Project and components

- **OHD** — Open Health Data. The project name and the protocol family.
- **OHD Storage** — the core data layer. Owns the on-disk format, persistence, permissions, audit, grants, and sync. Exposes the OHDC protocol. Runs in any of four deployment modes (on-device, OHD Cloud, custom provider, self-hosted).
- **OHD Connect** — the canonical personal-side application. Android, iOS, and web. Authenticated to OHDC under self-session. The user's primary tool for logging, viewing personal data, managing grants, reviewing pending submissions, and exporting.
- **OHD Care** — the canonical professional-side application. Reference, real, lightweight EHR-shaped consumer of OHDC. Multi-patient via grant tokens. Designed for OHD-data-centric clinical workflows; not a competitor to enterprise EHRs.
- **OHD Emergency** — the canonical emergency-services-side application. Reference, real, lightweight EHR-shaped consumer for paramedics, ambulance crews, ER triage, mobile-care services. Break-glass discovery via BLE; case-bound grants issued through certified-authority cert chain.
- **OHD Relay** — the bridging service. Forwards opaque packets between OHDC consumers and storage instances that can't accept inbound connections (phones, home servers behind NAT). Has optional **emergency-authority mode** (signs emergency-access requests with a certified authority cert). Sees ciphertext only.

## Protocol and auth

- **OHDC** — the OHD Connect protocol. The single external API of OHD Storage. All operations (read, write, aggregate, grant management, audit, export, pending review) live in OHDC; capability is determined by the session's auth profile.
- **Self-session** — auth profile for the user themselves, via OIDC. Full scope on own data.
- **Grant token** — auth profile for a third party with a user-issued grant. Bounded by the grant's structured rules.
- **Device token** — specialized grant: write-only, no expiry, attributed by `device_id`. Used by sensor / lab / pharmacy / EHR integrations and the user's own per-device write services. Modelled in the schema as a row in `grants` with `kind='device'`.
- **Approval mode** — per-grant write policy: `always` (every submission queues for user review), `auto_for_event_types` (pre-authorized types auto-commit), `never_required` (all submissions auto-commit; for trusted long-term grants and emergency).
- **Pending event** — a grant-submitted event awaiting user review under the approval queue. Lives in `pending_events` until promoted to `events` (approved) or marked rejected/expired.
- **Sensitivity class** — registry-level metadata on event types and channels (`general`, `mental_health`, `sexual_health`, `substance_use`, `reproductive`, `biometric`, `lifestyle`). Coarse hook for grant rules.
- **Emergency-template grant** — pre-configured grant in `grants` with `is_template=1`, `kind='emergency_template'`. Cloned into a fresh active grant when break-glass fires. Defines the user's emergency profile (which channels visible, history window, etc.).
- **Break-glass** — emergency access flow. A first responder's certified-authority relay sends a signed emergency-access request to the patient's phone; phone shows dialog above lock screen; user approves or 30s timeout fires with default-allow; on grant, an active emergency case opens.
- **Auto-granted access** — break-glass access that fired via timeout-default-allow rather than explicit user approval. Recorded in `audit_log.auto_granted=1`; rendered distinctly in the user's audit view.
- **Bystander proxy** — any OHD Connect installation can transparently relay BLE-encapsulated emergency requests to the responder's relay over its own internet connection. Bystander sees only TLS ciphertext.
- **Authority cert chain** — the cryptographic trust chain that authenticates an emergency-authority relay (EMS station, hospital, etc.) to the patient's phone. OHD project maintains a default trust root; sub-certs go to certified emergency services.
- **Case** — a labeled, curated container of events, defined by a list of filter expressions (`case_filters`). Has lifecycle (start, end, auto-close) and optional linkage to other cases. Owns no access rules — those live on grants. Events themselves are case-agnostic; cases find their events via filters at query time. Linkage is asymmetric: predecessor → successor (handoff context flows forward), children → parent (sub-case results roll up).
- **Case reopen token** — time-limited token issued to an authority when their case auto-closes from inactivity. Lets them resume the case within a TTL without re-running break-glass / patient approval.
- **Handoff** — transferring a case from one authority to another (e.g., EMS → ER). Opens a successor case linked to the current; closes current; previous authority retains read-only access to their span.

## Deployment modes

- **On-device** — OHD Storage runs inside the user's OHD Connect mobile/desktop app. The user is the operator; their data never leaves the device unless explicitly granted.
- **OHD Cloud** — OHD Storage runs on the OHD project's hosted SaaS. We're the operator; we never sell user data.
- **Custom provider** — OHD Storage runs on a third party's infrastructure (clinic, hospital, insurance company, employer wellness, research consortium). They're the operator.
- **Self-hosted** — OHD Storage runs on a server the user controls (VPS, NAS, home server). The user is the operator.

See [`deployment-modes.md`](deployment-modes.md) for tradeoffs and decision criteria.

## File and storage

- **`.ohd`** — file extension for the encrypted OHD Storage SQLite file.
- **`<file>.ohd-blobs/`** — sidecar directory for attachment payloads, encrypted with the same per-user key.
- **Per-user file** — one storage file per user. SaaS deployments shard across users (`users/<hash>/<user_ulid>/data.ohd`).
- **Sample block** — compressed `(t_offset, value)` stream stored as a BLOB row in `event_samples`. Default ~15-minute window per block. ~100× density gain over per-sample rows for dense series (HR, glucose, ECG).
- **Channel** — a typed numeric or categorical measurement, registered in the channel registry. Channels form a tree per event type.
- **Event** — one occurrence at a `(timestamp_ms, optional duration_ms)`, with an event_type, zero or more channel values, optional sample streams and attachments, source / device / app provenance.

## Identifiers

- **ULID** — Universal Lexicographically-sortable IDentifier. 128 bits = 48-bit unsigned ms epoch + 80-bit CSPRNG random. Used as the wire identifier for events, grants, attachments, and pending events.
- **`ulid_random`** — the 80-bit (10-byte) random portion stored on disk. The wire ULID is reconstituted from `(timestamp_ms, ulid_random)` at the API boundary.
- **Clamp-to-0** — for events with `timestamp_ms < 0` (pre-1970), the ULID's 48-bit time prefix is clamped to 0. The 80-bit random keeps identity unique. Sort by `timestamp_ms`, not by ULID, when mixing eras.
- **`user_ulid`** — the user's wire identity, stored in `_meta.user_ulid`. Mapped to OIDC `(provider, subject)` at the deployment level; no PII in the protocol.

## Registry

- **`std.*`** — namespace for the standard channel and event-type catalog. Ships with the format spec; identical across implementations.
- **`com.<owner>.<name>.*`** — namespace for custom event types and channels. Lives in the user's file alongside standard ones; round-trips through export/import.
- **Channel alias** — entry in `channel_aliases` mapping an old channel path to a current one. Resolves at read time; compactor lazily rewrites event_channels rows.
- **Type alias** — same shape as channel alias, for event types.

## Sync

- **Primary** — file-level role: canonical for the user. Accepts writes, serves external grant queries.
- **Cache** — file-level role: mirrors a remote primary. Local writes flush to primary; remote-origin events sync in.
- **Mirror** — file-level role: read-only replica. Backups, hot standbys.
- **`peer_sync` row** — per-peer watermarks (`last_outbound_rowid`, `last_inbound_peer_rowid`) tracking what's been sent and consumed.
- **`origin_peer_id`** — column on `events` marking which peer (if any) the row originated from. NULL means locally minted.

## Audit

- **`actor_type`** — `'self'` (user authenticated to themselves) / `'grant'` (third-party grantee, includes device tokens with `grants.kind='device'`) / `'system'` (deployment-level operations like sync import).
- **`rows_filtered`** — count of rows that matched a query but were silently stripped by grant rules. The grantee never sees this; the user always does, in their personal-side audit view.
- **`audit_retention_days`** — `_meta` key, default `NULL` (forever); finite values enable a background cleanup pass.

## Repos and binaries

| What | Repo / Package | Binary / Library |
|---|---|---|
| OHD Storage core (Rust) | `ohd-storage` | `libohdstorage` (Rust crate; uniffi/PyO3 bindings) |
| OHD Connect Android | `ohd-connect-android` | `.aar` |
| OHD Connect iOS | `ohd-connect-ios` | `.xcframework` |
| OHD Connect web | `ohd-connect-web` | static SPA |
| OHD Connect MCP | `ohd-connect-mcp` | `ohd-connect-mcp` binary |
| OHD Connect CLI | `ohd-connect-cli` | `ohd-connect` binary |
| OHD Care app | `ohd-care-app` | web SPA + small backend |
| OHD Care MCP | `ohd-care-mcp` | `ohd-care-mcp` binary |
| OHD Care CLI | `ohd-care-cli` | `ohd-care` binary |
| OHD Emergency tablet | `ohd-emergency-mobile` | Android `.aar` / iOS `.xcframework` |
| OHD Emergency dispatch | `ohd-emergency-dispatch` | web SPA + backend |
| OHD Emergency MCP | `ohd-emergency-mcp` | `ohd-emergency-mcp` binary |
| OHD Emergency CLI | `ohd-emergency-cli` | `ohd-emergency` binary |
| OHD Relay | `ohd-relay` | `ohd-relay` binary (with emergency-authority mode flag) |

## Wire and transport

- **Connect-RPC** — the OHDC protocol family. Schema-first via Protobuf; clients and servers generated by Buf CLI. Wire-compatible with gRPC. See [`components/connect.md`](components/connect.md) "Wire format".
- **Protobuf** — the OHDC schema language. Binary wire encoding (`application/proto`) by default; JSON encoding (`application/json`) available per-request for debugging. Same schema, two encodings.
- **Buf** — the Protobuf tooling chain used by OHD. `buf generate` for codegen, `buf curl` for ad-hoc testing, `buf breaking` for CI compatibility checks, Buf Schema Registry (`buf.build/openhealth-data/ohdc`) for published reference docs.
- **HTTP/3** — over QUIC. Default transport between consumers and storage. Caddy 2.6+ handles TLS and HTTP/3 termination on the operator side.
- **HTTP/2** — fallback for clients without HTTP/3 support.
- **TLS 1.3** — required for all OHDC traffic. SQLCipher 4 for at-rest encryption (Argon2id when SQLCipher 5 ships).
- **Wire path prefix** — `/ohdc.v0.OhdcService/<Method>` for OHDC operations (Connect-RPC convention), `/relay/v1/...` for relay routing.

## Things that are NOT terms (deliberate omissions)

- **OHDP / OHD Push** — earlier draft naming for "the write-only subset of OHDC." No longer used; superseded by **device token** as a kind of grant under the unified OHDC protocol.
- **CORD / OHD Cord** — earlier draft naming for "the read-only protocol / app." Renamed to **OHD Care** at the app level; the protocol is just OHDC.
- **OHD Core** — earlier name for OHD Storage. The component file `components/storage.md` was renamed from `ohd-core.md`. "Core" is now the role of OHD Storage, not a separate component name.
- **OHD TURN** — earlier draft naming for the bridge. Renamed to **OHD Relay** to avoid collision with WebRTC's TURN protocol.

## Cross-references

- Architecture overview: [`01-architecture.md`](01-architecture.md)
- Deployment modes (user-facing): [`deployment-modes.md`](deployment-modes.md)
- OHDC protocol + OHD Connect: [`components/connect.md`](components/connect.md)
- OHD Care: [`components/care.md`](components/care.md)
- OHD Emergency: [`components/emergency.md`](components/emergency.md)
- OHD Relay: [`components/relay.md`](components/relay.md)
- OHD Storage: [`components/storage.md`](components/storage.md)
- On-disk format: [`design/storage-format.md`](design/storage-format.md)
- OHDC v0 protocol (services, messages, error model, .proto): [`design/ohdc-protocol.md`](design/ohdc-protocol.md)
- Authentication (self-session, OIDC, tokens, account-join modes): [`design/auth.md`](design/auth.md)
- Care operator auth (clinic SSO, grant-token vault, two-sided audit): [`design/care-auth.md`](design/care-auth.md)
- Encryption & key management (file key, recovery, pairing, rotation): [`design/encryption.md`](design/encryption.md)
- Emergency trust (Fulcio + X.509, short-lived authority certs, signed requests, patient-phone verification): [`design/emergency-trust.md`](design/emergency-trust.md)
- Notification delivery (FCM/APNs/email push, no-PHI payloads): [`design/notifications.md`](design/notifications.md)
- Relay wire protocol (tunnel framing, cert pin, multiplexing): [`design/relay-protocol.md`](design/relay-protocol.md)
- Sync wire protocol (cache↔primary, SyncService, watermarks, attachment sync): [`design/sync-protocol.md`](design/sync-protocol.md)
- Conformance corpus (test fixtures every OHDC v0 implementation must pass): [`design/conformance.md`](design/conformance.md)
- Privacy / access control: [`design/privacy-access.md`](design/privacy-access.md)
- Device pairing (deferred, post-v1 — design-space sketch): [`future-implementations/device-pairing.md`](future-implementations/device-pairing.md)
- Conceptual event vocabulary: [`design/data-model.md`](design/data-model.md)
- Operator deployment (Docker / Caddy / Hetzner): [`design/deployment.md`](design/deployment.md)
