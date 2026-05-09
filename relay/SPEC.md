# OHD Relay — Implementation Spec

> Implementation-ready distillation of the canonical [`relay component spec`](../spec/docs/components/relay.md) and [`relay-protocol`](../spec/docs/design/relay-protocol.md), focused on what *this crate* needs to ship. Cross-cutting concerns (OHDC payload format, grant token shape, encryption key hierarchy) reference back to `../spec/`.

This doc is the local contract for the implementation agent. When in doubt, the canonical sources in `../spec/` win — but everything below should be true to them.

## Scope

The relay is a **Rust binary** that:

1. Accepts HTTP/3 (QUIC) connections from OHD Storage instances and assigns each a stable rendezvous ID.
2. Accepts HTTP/3 connections from OHDC consumers at `https://<host>/r/<rendezvous_id>` and routes them to the matching storage tunnel.
3. Multiplexes many consumer sessions onto one storage tunnel using a small framed protocol (see "Frame format" below).
4. Sees only ciphertext. TLS terminates at consumer and at storage — end-to-end through the tunnel.
5. Holds **five-field state per registered user** plus per-session counters; persists nothing else.
6. Optionally — feature-gated — operates in **emergency-authority mode**: holds a Fulcio-issued 24h authority cert, signs `EmergencyAccessRequest` payloads for break-glass.

The relay is fronted by Caddy for outer TLS (the cert that authenticates `relay.example.com`). Caddy proxies QUIC + HTTP/3 down to this binary on a local port.

## Out of scope (what the relay must NOT do)

- **Does not decrypt** OHDC traffic. The inner TLS handshake — consumer ↔ storage — is opaque to the relay.
- **Does not authenticate the OHDC payload.** Storage validates the token (self-session / grant / device) on every RPC. A leaked relay session credential is a forwarding tube, not data access.
- **Does not store data.** Sessions are ephemeral; logs are operational telemetry, not health data.
- **Does not enforce grant scope.** That's storage's job behind the relayed TLS.
- **Does not replicate state across multiple relays** in v1. One user, one active relay registration. Future revisions may add forwarding pointers.
- **Does not modify, replay, or buffer DATA frames** beyond what's needed for flow control / forwarding. Frame payloads are forwarded byte-for-byte after demux.

## Routing patterns (recap)

### Pairing-mediated (ephemeral, in-person)

Trust anchor: physical proximity. Used for short doctor-at-desk sessions.

1. Phone and operator's device exchange a **pairing nonce** out-of-band (NFC tap or QR).
2. Phone calls `POST /relay/v1/pair` with the pairing nonce → relay returns a one-shot rendezvous URL and a per-pairing credential.
3. Operator's client connects to the rendezvous URL, presents the same pairing nonce.
4. Frames flow as in grant-mediated.
5. Pairing credential expires when the session ends or after **30 minutes of inactivity**.

### Grant-mediated (durable, remote)

Trust anchor: the user's grant token. Used for home servers and for phones that want to be remotely reachable.

1. Storage calls `POST /relay/v1/register` with a one-time registration token (obtained from the relay's setup web UI). Provides Ed25519 SPKI, push token (optional), short label.
2. Relay returns a stable opaque `rendezvous_id`, `rendezvous_url`, and a `long_lived_credential` for tunnel auth.
3. Storage opens the long-lived tunnel via `RelayService.OpenTunnel` (`POST /relay/v1/tunnel`) authenticated with `long_lived_credential`. Tunnel is bidi-streaming Connect-RPC carrying binary `TunnelFrame` chunks.
4. Storage sends `HELLO`; relay replies `HELLO`. Heartbeat `PING/PONG` every **25s**.
5. Consumer arrives at `https://<host>/r/<rendezvous_id>`, presents grant token in its `HELLO`. Relay assigns a fresh `SESSION_ID`, sends `OPEN` to storage; storage replies `OPEN_ACK` (or `OPEN_NACK` with reason).
6. Inner TLS handshake (consumer ↔ storage) flows as `DATA` frames. Relay forwards bytes opaquely.
7. OHDC traffic rides the inner TLS session.
8. Either side can `CLOSE` a session; storage tunnel close drops all sessions and flips registration to `reconnecting`.

## Five-field per-user state

The relay's privacy property — "compromise reveals only traffic patterns" — depends on this minimalism.

```
(rendezvous_id, user_ulid, current_tunnel_endpoint, push_token, last_heartbeat_at)
```

Plus a small log of recent connection events for operational telemetry. **Nothing else** — no grants, no audit, no tokens, no PHI.

## In-memory tables

The implementation maintains three core tables (`src/state.rs`):

### `RegistrationTable`

Durable. One row per registered user. v1 storage backend: SQLite (durable across relay restarts).

```text
RegistrationRow {
  rendezvous_id:           String,        // opaque, ~22-char base32, NOT derived from user_ulid
  user_ulid:               Ulid,          // 128-bit
  current_tunnel_endpoint: Option<EndpointHandle>,  // None when storage offline
  push_token:              Option<PushToken>,       // FCM | APNs | none
  last_heartbeat_at_ms:    i64,
  long_lived_credential_hash: [u8; 32],   // tunnel-auth bearer; verify hash, never log raw
  registered_at_ms:        i64,
  user_label:              Option<String>,          // opaque short label
  storage_pubkey:          Ed25519PubKey,           // for sanity check on re-registration
}
```

Constraints:

- `UNIQUE(rendezvous_id)`.
- One active registration per `(relay, user_ulid)` in v1.
- Garbage-collect rows after **30 days** of no successful tunnel-open.

### `SessionTable`

In-memory only. One row per active consumer-attach.

```text
SessionRow {
  session_id:        u32,                      // assigned by relay; unique per tunnel
  rendezvous_id:     String,                   // FK to RegistrationTable
  attached_at_ms:    i64,
  last_active_at_ms: i64,
  bytes_in:          u64,
  bytes_out:         u64,
  flow_control:      FlowControlState,         // 256 KB default receive window per side
}
```

Closed sessions are evicted; metering counters can fold up into per-user totals.

### `PairingTable`

In-memory, ephemeral. One row per outstanding pairing nonce.

```text
PairingRow {
  nonce:               String,                 // opaque, single-use
  expires_at_ms:       i64,                    // typically created + 30 min
  attached_session_id: Option<u32>,
  per_pairing_credential_hash: [u8; 32],
}
```

## Wire protocol (recap from `spec/relay-protocol.md`)

### Frame format (binary, big-endian)

```
0       1       2       3       4
+-------+-------+-------+-------+
| MAGIC | TYPE  | FLAGS | RSVD  |   MAGIC = 0x4F ('O')
+-------+-------+-------+-------+
| SESSION_ID (4 bytes, BE u32)  |   0 = control / unbound
+-------+-------+-------+-------+
| PAYLOAD_LEN (4 bytes, BE u32) |   max 65535
+-------+-------+-------+-------+
| PAYLOAD (PAYLOAD_LEN bytes)   |
+-------+-------+-------+-------+
```

### Frame types

| TYPE | Name | Direction | Notes |
|---|---|---|---|
| `0x01` | `HELLO` | Both | Capability negotiation |
| `0x02` | `OPEN` | Relay → Storage | New consumer attach; payload includes consumer's grant-token preview |
| `0x03` | `OPEN_ACK` | Storage → Relay | Storage accepts; ready for DATA |
| `0x04` | `OPEN_NACK` | Storage → Relay | Storage rejects (`INVALID_TOKEN`, `RATE_LIMITED`, ...) |
| `0x05` | `DATA` | Both | Opaque ciphertext (TLS records after inner handshake) |
| `0x06` | `CLOSE` | Both | Tear down `SESSION_ID`; payload optional reason code |
| `0x07` | `PING` | Both | Keepalive |
| `0x08` | `PONG` | Both | Reply to `PING` |
| `0x09` | `WAKE_REQUEST` | Relay → Storage (out-of-band via push) | Not a tunnel frame; FCM/APNs payload only |
| `0x0A` | `WINDOW_UPDATE` | Both | Per-session flow control |
| `0x80..0xFF` | Reserved | — | Vendor / experimental |

The implementation's frame codec is the most-load-bearing single module. Spec target: ~500 lines of Rust for parser + serializer + dispatch loop.

### Per-session flow control

256 KB receive window per side per session, updated by `WINDOW_UPDATE`. Per-session, not per-tunnel — one slow consumer must not starve others.

## RPC surface

All paths are under `/relay/v1/...`. Connect-RPC over HTTP/3 (with HTTP/2 fallback for tooling). The full schemas live in OHDC's `relay_service.proto` (out of scope for this crate's repo; see `../spec/docs/design/relay-protocol.md` and `ohdc-protocol.md`).

| RPC | Path | Auth | Purpose |
|---|---|---|---|
| `RelayService.Register` | `POST /relay/v1/register` | One-time registration token (Bearer) | First-time storage registration |
| `RelayService.OpenTunnel` | `POST /relay/v1/tunnel` (bidi stream) | `long_lived_credential` (Bearer) | Storage's persistent tunnel |
| `RelayService.RefreshRegistration` | `POST /relay/v1/refresh` | `long_lived_credential` | Update push token / label |
| `RelayService.Heartbeat` | `POST /relay/v1/heartbeat` | `long_lived_credential` | Registration-level keepalive (independent of tunnel `PING`) |
| `RelayService.Deregister` | `POST /relay/v1/deregister` | `long_lived_credential` | Drop registration; remove rendezvous record |
| Consumer attach | `CONNECT /r/<rendezvous_id>` | Grant token in `HELLO` payload (validated by storage) | Consumer-side rendezvous |
| Pair init | `POST /relay/v1/pair` | None / pairing nonce | Pairing-mediated short-lived rendezvous |
| Setup UI | `GET /setup` | Operator-config | Issues one-time registration tokens |

The skeleton (`src/server.rs`) stubs these handlers as `unimplemented!()` placeholders; implementation phase wires them up.

## Persistence

| What | Where | Notes |
|---|---|---|
| `RegistrationTable` | SQLite at `relay.db` (path from config) | Survives restarts; durable per-user state |
| `SessionTable` | RAM only | Lost on restart; consumers reconnect |
| `PairingTable` | RAM only | Short-lived |
| Bandwidth meter / per-user counters | SQLite, periodically flushed | For billing / abuse, not the wire path |
| Operator-defined access lists (clinic-run pre-shared keys, etc.) | Config file | See `deploy/relay.example.toml` |

Horizontal scaling: shard by `rendezvous_id` (consistent hash) so all sessions for a given user pin to one instance. Cross-instance forwarding is not v1.

## Phone-storage push-wake

When a consumer attaches but `current_tunnel_endpoint` is `None` (phone asleep):

1. Relay queues the consumer attach in `PendingAttach { rendezvous_id, started_at_ms }`.
2. Relay sends a **silent push** to the registration's `push_token`:
   - FCM: data-only message, `priority=high`, no `notification` key.
   - APNs: background notification, `apns-priority: 10`.
   - Payload: `{"category":"tunnel_wake","ref_ulid":"<rendezvous_id>"}` — **no PHI**.
3. Connect mobile wakes, opens a fresh `OpenTunnel` stream (using the same `long_lived_credential`), the relay completes the consumer's `OPEN` once the tunnel is up.
4. If the tunnel doesn't come up within ~5s the consumer sees a transient `503 STORAGE_OFFLINE`; consumer-side retry policy applies.

The wake path is the **only** time the relay touches push providers. Push secrets live in the relay's deploy config; see `deploy/relay.example.toml`.

For full payload contract see [`spec/notifications.md`](./spec/notifications.md). The tunnel-wake category is the silent variant; emergency-access push (the loud one) is sourced by the storage's notification dispatcher, not the relay — even in authority mode.

## Emergency-authority mode (feature-gated)

Build with `--features authority` to enable. Without the feature, the relay is a plain packet forwarder and the auth-mode code is dead-stripped.

A relay in authority mode additionally:

- **Holds an authority cert chain** issued by an OHD-trusted Fulcio: `[org_cert, fulcio_intermediate, ohd_root]`. Cert TTL is 24h; daily refresh is automated.
- **Onboarding (one-time, human-mediated)**: org's master pubkey registered as an authorized client of the OHD emergency-authority OIDC IdP.
- **Daily refresh** via Fulcio's standard `POST /api/v2/signingCert` endpoint with an OIDC bearer + Ed25519 proof-of-possession. Use `sigstore-rs` or equivalent — no custom Fulcio client.
- **Accepts inbound emergency requests** from authenticated responders. Responder auth is operator policy (clinic SSO / hospital ADFS / paramedic-roster auth) — not part of the OHD protocol. The relay must verify the responder is currently a member of the operator's roster.
- **Signs the `EmergencyAccessRequest`** Protobuf with the leaf cert's private key (Ed25519 over canonical encoding with the `signature` field zeroed). Standard X.509 detached-signature shape.
- **Routes responder traffic** post-grant exactly like any other consumer attach. The signing duty is an *additional* responsibility; the forwarding role is unchanged.
- **Maintains operator-side audit** of every responder break-glass attempt and outcome. This is in addition to the patient-side audit OHD records.

Authority-mode trust hierarchy (see `spec/emergency-trust.md` for the full diagram):

```
OHD Project Root CA (10y, offline HSM)
  └── OHD Global Fulcio (1y intermediate, online)        ← refreshes via /api/v2/signingCert
        └── Org cert (24h, this relay's daily-refresh leaf)
              └── Responder cert (1-4h, optional, for per-shift accountability)
```

The crate stub:

- `src/auth_mode.rs` — `#[cfg(feature = "authority")]` module. Stubs `AuthorityCertChain`, `EmergencyRequestSigner`, `OidcRefreshClient`. No live network calls in the scaffold.
- `Cargo.toml` declares `[features] authority = []` and the (currently empty) dep set guarded by it.

## Trust model (operational)

| Threat | Mitigation |
|---|---|
| Passive eavesdropping at relay | Inner TLS 1.3; relay sees ciphertext only |
| Malicious relay operator | Same as above; relay can DoS but not read or forge |
| Forged session attach | Pairing nonces are short-lived; grant tokens validated by storage on `OPEN` |
| Stolen `long_lived_credential` | Storage rotates by deregistering and re-registering; relay verifies hash, never echoes the raw value |
| Compromised authority cert | 24h TTL caps blast radius; emergency deny-list is a v1.x lever |
| Subpoena at relay | Operational metadata recoverable; payloads not |

## Operational targets

- **Heartbeat at tunnel level (`PING`)**: every 25s.
- **Tunnel teardown after**: 60s of `PING` silence.
- **Reconnect backoff (storage side)**: 1s → 2s → 4s → 8s → 16s → 30s cap, indefinite.
- **Pairing TTL**: 30 min idle.
- **Registration GC**: 30 days no successful tunnel-open.
- **Per-session receive window**: 256 KB default.
- **Max frame payload**: 65,535 bytes.
- **Implementation effort**: ~500 LOC for the tunnel layer; ~2k LOC for the binary including HTTP handlers, metering, telemetry.

## Cross-references

- Component overview: [`../spec/docs/components/relay.md`](../spec/docs/components/relay.md)
- Wire spec (vendored): [`./spec/relay-protocol.md`](./spec/relay-protocol.md)
- Authority cert mechanism (vendored): [`./spec/emergency-trust.md`](./spec/emergency-trust.md)
- Push-wake payload contract (vendored): [`./spec/notifications.md`](./spec/notifications.md)
- OHDC general protocol (what flows over the inner TLS): `../spec/docs/design/ohdc-protocol.md`
- Storage's identity key + cert renewal: `../spec/docs/design/encryption.md`
- Glossary: `../spec/docs/glossary.md`

## Open items (carried forward from the spec)

- LAN fast-path NAT traversal on networks that block mDNS (clinic / enterprise).
- Per-grant sub-rendezvous for compartmentalization (deferred).
- Multi-relay simultaneous registration with forwarding pointers (deferred).
- Per-grant revocation propagation through active sessions (relay-side signaling vs. plain TCP RST).
- Concrete BLE service UUID / characteristics for bystander-mediated emergency transport (deferred to integration phase).
