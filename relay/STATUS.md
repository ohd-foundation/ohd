# OHD Relay — Implementation Handoff

> Status of this crate at end of the implementation pass. Read alongside
> [`SPEC.md`](./SPEC.md) before picking up further work.

## Emergency-flow endpoints landed (2026-05-09)

Two new HTTP endpoints close the operator-tablet break-glass loop:

- `GET  /v1/emergency/status/{request_id}` — poll the state of an
  in-flight emergency-access request (waiting / approved / rejected /
  auto_granted_timeout / expired). Available in **all** builds (not
  feature-gated; the tablet polls regardless of authority mode).
- `POST /v1/emergency/handoff` — hand off an active break-glass case to
  a successor operator (e.g. EMS → Motol ER). Forwards
  `OhdcService.HandoffCase` through the storage tunnel and records the
  operator-side audit row.

Implementation lives in **`src/emergency_endpoints.rs`** (new module) +
two route lines in `src/server.rs::build_router`. Schema migration adds
`_emergency_requests` and `_emergency_handoffs` tables to the
relay-local SQLite (idempotent — see `state.rs::init_schema`).

### State machine

`POST /v1/emergency/initiate` (authority-mode) inserts a `_emergency_requests`
row with `state=waiting`, `expires_at_ms=now+30s`, and the patient's
emergency-profile `default_action` if reachable via the storage tunnel.
The state machine then transitions on:

- **patient phone responds** → relay's notification handler calls
  `EmergencyStateTable::approve_request` / `reject_request` to flip the
  row. `grant_token` + `case_ulid` (from the patient's response) get
  populated on approve.
- **TTL elapses without a response** → background sweeper task
  (`run_ttl_sweeper_loop`, kicked off from `run_serve`) reads
  `default_action` from the persisted row:
  - `Allow` → `auto_granted_timeout` with a freshly-minted grant token.
  - `Deny` (or NULL — fail-closed when the relay couldn't reach the
    patient's emergency profile) → `expired`.
- **TTL grace + 5min** → row GC'd by the same sweeper.

### `POST /v1/emergency/handoff` flow

1. Resolve the patient rendezvous (request body or last-known via
   `_emergency_handoffs`).
2. Look up the registration; reject with 404 + `code=rendezvous_unknown`
   if the case isn't active under this relay's authority.
3. Forward the handoff event via `StorageTunnelClient::handoff_case`,
   which the storage receives as `OhdcService.HandoffCase` over the
   inner-TLS-through-tunnel.
4. Storage returns `(successor_case_ulid, successor_operator_label,
   predecessor_read_only_grant)`.
5. Record the handoff in `_emergency_handoffs` with a fresh
   `audit_entry_ulid`; return all four fields to the tablet.

The wire response includes both `predecessor_read_only_grant` (the
prompt's field name) and `read_only_grant_token` (the existing tablet
DTO's name) so the tablet works without an in-flight code change.

### Storage tunnel client (new trait)

`StorageTunnelClient` is the relay's outbound abstraction for
`OhdcService.HandoffCase` and `GetEmergencyConfig`. Production wiring
(real Connect-RPC over the tunnel) is the storage-side outbound
deliverable tracked under "What's stubbed / TBD"; until it lands the
relay binary's `AppState.storage_tunnel` is `None` and `/v1/emergency/handoff`
returns 503 with `code=storage_tunnel_unavailable`. Tests use
`MockStorageTunnel` which canned-responds keyed by `rendezvous_id`.

### Defaults

- `DEFAULT_REQUEST_TTL = 30s` — relay-side wait window between
  initiate and patient response. Distinct from the **signed payload's**
  own `expires_at_ms` (5 min — when the patient phone considers the
  signature stale).
- `REQUEST_GC_GRACE = 5min` — added to `expires_at_ms` to compute
  `gc_after_ms`.
- `TTL_SWEEPER_TICK = 1s` — sweeper poll cadence.

### Tests

- **Unit (`src/emergency_endpoints.rs::tests`)** — 13 tests: schema
  migration, insert/lookup roundtrip, state transitions (approve /
  reject / cannot-double-transition), TTL sweep auto-grant /
  expire / no-profile fail-closed, GC, handoff insert/lookup,
  MockStorageTunnel availability + unavailable + canned-error paths,
  state-string roundtrips.
- **Integration (`tests/end_to_end_emergency_endpoints.rs`)** — 7
  tests over the real axum router: status 404 on unknown request, full
  flow (initiate-equivalent → status waiting → simulate approval →
  status approved → handoff → audit row in DB), handoff 503 when
  tunnel unconfigured, handoff 404 on unknown rendezvous, TTL sweep
  auto-grant + sweep expire surfaced via HTTP, `record_initiated_request`
  pulls patient label + default action from `GetEmergencyConfig` when
  the tunnel is up.

## OHDC wire/API version renamed to v0 (2026-05-09)

Relay-facing OHDC proto examples and docs now use `ohdc.v0` for the
storage-owned pre-stable API namespace. Relay's own REST `/v1/*` endpoints
remain unchanged because they are not OHDC Connect-RPC service paths.

## What's working now

A buildable, runnable, integration-tested relay binary:

- `cargo build` succeeds clean. `cargo build --features authority` adds the
  Fulcio + X.509 + Ed25519 dep stack and also builds clean.
- `cargo test` runs **103 tests** (80 unit + 4 e2e + 7 emergency
  endpoints + 1 HTTP/3 + 8 OIDC gating + 3 raw QUIC tunnel) — all green.
- `cargo test --features authority` runs **124 tests** (97 unit + 4 e2e
  + 4 emergency sign/verify + 7 emergency endpoints + 1 HTTP/3 + 8 OIDC
  gating + 3 raw QUIC tunnel) — all green.
- `cargo run -- health` and `cargo run -- version` work as before.
- `cargo run -- serve --db /tmp/ohd-relay.db --port 8443
  --config relay.toml` starts the axum HTTP server. Push providers (FCM /
  APNs) and authority-mode wiring come from `relay.toml` per
  `deploy/relay.example.toml`.

The end-to-end test exercises:
1. Storage POSTs `/v1/register` with hex ULID + storage pubkey + label.
2. Relay returns `(rendezvous_id, rendezvous_url, long_lived_credential)`.
3. Storage opens a `WS /v1/tunnel/:rid` connection.
4. Consumer opens a `WS /v1/attach/:rid` connection.
5. Relay assigns a fresh `session_id`, sends `OPEN` to storage; storage
   acks with `OPEN_ACK`.
6. Consumer pushes a 4 KiB `DATA` frame → byte-identical delivery to storage.
7. Storage pushes a 4 KiB `DATA` frame → byte-identical delivery to consumer.
8. Consumer sends `CLOSE`; storage observes `CLOSE` for the same `session_id`.

## Modules landed

| Module | What's there | Tests |
|---|---|---|
| `src/frame.rs` | Binary `TunnelFrame` codec, all 9 frame types, encode/decode + boundary checks | 14 unit |
| `src/state.rs` | SQLite-backed `RegistrationTable` (rusqlite + spawn_blocking), schema, recent-events log | 5 unit |
| `src/pairing.rs` | In-memory `PairingTable` with TTL expiry + sweeper task | 5 unit |
| `src/session.rs` | `TunnelEndpoint` + `SessionTable` + per-session mpsc channels | 3 unit |
| `src/push/mod.rs` | `PushClient` trait, `PushDispatcher` (token-type router) | 2 unit |
| `src/push/fcm.rs` | **Real FCM HTTP v1 client** — service-account JSON load, in-process OAuth2 jwt-bearer minting (`jsonwebtoken` RS256), `messages:send` POST with retry/backoff, 401 → bearer refresh, 404/UNREGISTERED → InvalidToken | 5 unit |
| `src/push/apns.rs` | **Real APNs HTTP/2 client** — `.p8` ES256 JWT signer, `api.push.apple.com/3/device/<token>` POST with `apns-topic`/`apns-priority`/`apns-push-type`/`apns-id` headers, sandbox vs production, `ApnsUrgency::Background` for tunnel-wake / `Critical` for emergency | 5 unit |
| `src/config.rs` | `relay.toml` loader (TOML); `[push.fcm]`, `[push.apns]`, `[authority]`, `[auth.registration]` sections | 7 unit |
| `src/auth/mod.rs` | Re-exports the OIDC verifier types | n/a |
| `src/auth/oidc.rs` | `OidcVerifier` — JWKS-cache-backed `id_token` verifier (signature + exp/nbf + aud + issuer-allowlist), key-rotation refresh, mock-IdP test stub | 8 unit |
| `src/server.rs` | Axum router; `/v1/register` (with OIDC gating), `/v1/heartbeat`, `/v1/deregister`, `/v1/auth/info`, `WS /v1/tunnel/:rid`, `WS /v1/attach/:rid`, `GET /v1/emergency/status/:request_id`, `POST /v1/emergency/handoff`, `/health`; under `--features authority` also `POST /v1/emergency/initiate` | 4 e2e + 8 OIDC e2e + 7 emergency e2e |
| `src/emergency_endpoints.rs` | `_emergency_requests` + `_emergency_handoffs` access, state machine + transitions, TTL sweeper, `StorageTunnelClient` trait + `MockStorageTunnel`, status + handoff handlers | 13 unit + 7 e2e |
| `src/auth_mode/mod.rs` (feature-gated `authority`) | Re-exports submodule types; `AuthorityError` variant taxonomy | n/a |
| `src/auth_mode/cert_chain.rs` | `AuthorityCertChain` — PEM bytes + parsed leaf metadata + Ed25519 keypair; `is_current` / `millis_until_expiry` / `parse_leaf_validity` (via `x509-parser`) | 3 unit |
| `src/auth_mode/fulcio.rs` | Standard Sigstore Fulcio v2 `signingCert` client; OIDC + Ed25519 proof-of-possession, returns parsed cert chain | 4 unit |
| `src/auth_mode/rekor.rs` | Minimal Rekor v1 `intoto` log-entry submitter (soft-fail) | 3 unit |
| `src/auth_mode/refresh.rs` | `AuthorityState` cert-chain holder + `run_refresh_loop` background task (refresh ~1h before expiry; retry every 5m on failure) | 1 unit |
| `src/auth_mode/signer.rs` | `EmergencyAccessRequest` wire shape, `sign_request` / `verify_request` (chain validation + Ed25519 detached signature over canonical JSON), `MAX_CHAIN_DEPTH=4`, OHD emergency-authority EKU OID | 6 unit + 4 integration |
| `src/main.rs` | clap CLI: `serve`, `health`, `version`; `--config`/`--db`/`--port`/`--bind`/`--http3-*` flags | n/a |
| `src/lib.rs` | Re-exports modules so integration tests can drive the server | n/a |

## Pinned stack (decisions made this pass)

- **HTTP**: `axum 0.7` + `hyper 1` + `tokio` for HTTP/1.1 + HTTP/2 +
  WebSockets. Caddy fronts the binary in production and terminates outer
  TLS / HTTP/3.
- **WebSockets** (instead of bidi Connect-RPC): chose plain WS for the
  tunnel + attach paths because the wire is binary `TunnelFrame`s, not
  Protobuf, and axum's WS API is the shortest path. A future revision can
  swap to Connect-RPC bidi-streaming without changing the frame codec.
- **SQLite**: `rusqlite 0.31` with the `bundled` feature (statically linked
  SQLite). Encryption-at-rest is intentionally **not** used — relay sees
  ciphertext only, so SQLCipher would be theatre.
- **Random + hash**: `rand 0.8` for rendezvous IDs and credentials; `sha2`
  for credential-hash; constant-time compare hand-rolled (32-byte fixed).
- **HTTP client (push + Fulcio + Rekor)**: `reqwest 0.12` with
  `rustls-tls` feature — no OpenSSL link, single TLS stack across the
  binary.
- **JWT signing**: `jsonwebtoken 9`. Used for Google service-account
  RS256 (FCM OAuth2 jwt-bearer flow) and APNs ES256.
- **OAuth2 (FCM)**: hand-rolled jwt-bearer flow against
  `oauth2.googleapis.com/token`. We deliberately do NOT pull in
  `gcp_auth` / `yup-oauth2` — they each drag ~50 transitive deps for
  what is ~80 lines of `jsonwebtoken` + `reqwest`.
- **Authority cert chain**: `x509-parser 0.16` (with `verify` feature)
  for chain validation; `ed25519-dalek 2` for signing; `pem 3` for
  PEM en/decoding; `base64 0.22` for SPKI / signature wire encoding.
  We deliberately do NOT depend on the `sigstore` crate — it's
  experimental and pulls in tonic + a wide tree for surface we don't
  use. The Fulcio v2 client is ~150 lines of `reqwest` + `serde`.

## Wire-protocol implementation notes

- Frame format follows `spec/relay-protocol.md` exactly: 12-byte header
  (MAGIC + TYPE + FLAGS + RSVD + SESSION_ID + PAYLOAD_LEN), big-endian
  for `SESSION_ID` and `PAYLOAD_LEN`. `MAX_PAYLOAD_LEN == 65 535`.
- `0x09 WAKE_REQUEST` is **rejected** on the tunnel as `UnknownType` —
  that frame type is push-payload-only per the spec, not a tunnel frame.
- The codec rejects: bad magic, unknown frame types, non-zero RSVD byte,
  truncated header, truncated payload, payload-length over 65 535, and
  control frames (HELLO/PING/PONG) carrying a non-zero `SESSION_ID`.
- `TunnelFrame::decode` is strict (rejects trailing bytes); use
  `TunnelFrame::decode_one` for streaming-style consumption that returns
  the byte count consumed.

## Routing model (current implementation)

- One **TunnelEndpoint** per registered storage, keyed by `rendezvous_id`,
  registered in `SessionTable` when the storage's WS handshake completes.
- Per-tunnel: an mpsc `outbound_tx` queue (256 frames) drained by the
  writer task; a sessions map keyed by `session_id`.
- Consumer attach: relay allocates a `session_id`, installs a
  `storage_to_consumer_tx` in the per-endpoint sender map, sends `OPEN` to
  storage. Storage's `OPEN_ACK` is logged and consumed at the relay layer.
  Subsequent `DATA(session_id, ...)` frames from storage are routed to the
  matching consumer.
- Consumer's outbound bytes ride the WS as raw `TunnelFrame`s; the relay
  re-stamps `session_id` to its assigned value and forwards.
- `CLOSE` from either side tears down only that session.
- Storage tunnel disconnect drains every session and clears the
  registration's "tunnel up" flag.

## Push-wake path

`run_consumer_attach` calls `wait_for_tunnel`, which:
1. Looks up the tunnel; returns immediately if up.
2. Calls `push.wake(rendezvous_id, push_token)` — real FCM/APNs HTTP call
   if the matching backend is configured in `relay.toml`. Returns
   `UnsupportedTokenType` when no backend is wired (legacy / dev flow).
3. Polls every 50 ms up to `WAKE_DEADLINE` (5 s) for the storage to
   reconnect.
4. Returns `None` on timeout; the consumer attach handler then sends a
   final `CLOSE(STORAGE_OFFLINE)` and disconnects.

### Wire shape (what actually goes over the wire now)

**FCM HTTP v1**:
```
POST https://fcm.googleapis.com/v1/projects/<project_id>/messages:send
Authorization: Bearer <oauth2_access_token>
Content-Type: application/json

{
  "message": {
    "token": "<device_fcm_token>",
    "data": { "category": "tunnel_wake", "ref_ulid": "<rendezvous_id>" },
    "android": { "priority": "high" },
    "apns":    { "headers": { "apns-priority": "10",
                              "apns-push-type": "background" } }
  }
}
```

**APNs HTTP/2**:
```
POST https://api.push.apple.com/3/device/<device_token>
authorization: bearer <es256_jwt>
apns-topic: <bundle_id>
apns-push-type: background
apns-priority: 5
apns-id: <uuid>
Content-Type: application/json

{ "aps": { "content-available": 1 },
  "category": "tunnel_wake", "ref_ulid": "<rendezvous_id>" }
```

Critical Alert variant (authority mode only): `apns-push-type: alert` +
`apns-priority: 10` + body `{"aps":{"alert":..., "sound":"critical",
"interruption-level":"critical"}}`. Requires Apple's Critical Alert
entitlement on the OHD Connect bundle — Apple-issued, applied for
separately. Without the entitlement Apple silently downgrades to a
normal alert.

### Retry + token-rotation policy

Both clients implement the same retry shape:

- **2xx** → success.
- **401 / 403** (FCM `auth`, APNs `ExpiredProviderToken`) → invalidate
  cached bearer/JWT, refresh once, retry once. If the second call still
  fails auth, surface as `PushError::Auth`.
- **404 / 410 / `BadDeviceToken` / `UNREGISTERED` / `INVALID_ARGUMENT`** →
  `PushError::InvalidToken`. Caller marks the registration's token dead
  in the registration table (TODO — currently logged only; the spec's
  push_tokens.invalid_at_ms lives on storage, not relay).
- **429 / 5xx** → exponential backoff (250ms / 1s / 4s, capped),
  honoring `Retry-After` when FCM sets it. 3 attempts total.
- **anything else** → `PushError::Provider { status, body }`.

OAuth2 access tokens (FCM) are cached until 5 min before
`expires_in`. APNs JWTs are regenerated every 50 min (Apple's hard
ceiling is 1h).

## Per-OIDC registration gating (2026-05-09)

Per-relay-operator OIDC issuer allowlist for the registration RPC.
Three deployment shapes:

- **Permissive** (default, no `[auth.registration]` block): anyone who
  reaches `POST /v1/register` can register; `id_token` field ignored.
  Backwards-compatible with the original spec.
- **Hard-gated** (`require_oidc=true`): storage MUST present a valid
  `id_token` from one of the listed issuers, with the configured
  `expected_audience`. The OHD Cloud relay + clinic-run relays use
  this shape.
- **Soft-gated** (`require_oidc=false`, allowlist set): tokens are
  verified when present but registration without a token still
  succeeds. Useful during a clinic's rollout / migration window.

### What's wired

- **Config**: `[auth.registration]` block in `relay.toml` with
  `allowed_issuers` (issuer URL + expected audience), `require_oidc`,
  `jwks_cache_ttl_secs`. Parsed in `src/config.rs`; permissive is the
  empty-block default.
- **Verifier**: `src/auth/oidc.rs::OidcVerifier`. JWKS cache keyed by
  issuer URL with the configured TTL; on `kid` miss within TTL, one
  forced refresh (key rotation handling). Uses `jsonwebtoken 9` with
  `DecodingKey::from_jwk` so RS/ES/EdDSA keys all work; HMAC + `none`
  are rejected.
- **Discovery**: standard OIDC flow —
  `GET <issuer>/.well-known/openid-configuration` →
  `jwks_uri` → JWKS JSON.
- **Audit columns**: `registrations` SQLite table gains nullable
  `oidc_iss` + `oidc_sub` columns (Migration 001, idempotent
  `ALTER TABLE` in `src/state.rs::apply_migration_001_oidc_columns`).
  Filled in from the verified token's claims; NULL for permissive
  registrations.
- **Register handler**: `src/server.rs::handle_register` calls
  `enforce_registration_oidc` before persisting. Two precise reject
  paths: `OIDC_REQUIRED` (no token presented when require_oidc=true)
  and `OIDC_VERIFY_FAILED` (token present but invalid; reason in body).
- **Discovery endpoint**: `GET /v1/auth/info`, public, surfaces the
  allowlist + require_oidc bit so storage's relay-onboarding flow can
  pre-display "log in with X" buttons.

### Tests

- **Unit (`src/auth/oidc.rs`)**: 8 tests — valid token round-trip
  against a mock IdP, unknown-issuer reject, expired-token reject,
  audience-mismatch reject, bad-signature reject, kid-rotation
  handling (cache refresh on kid-miss), missing-token error, and the
  HMAC/none alg rejection.
- **State (`src/state.rs`)**: 1 test asserting OIDC columns persist on
  insert + lookup.
- **Config (`src/config.rs`)**: 3 tests — defaults are permissive,
  `[auth.registration]` parses correctly, `jwks_cache_ttl_secs`
  defaults to 3600 when only the block is present.
- **Integration (`tests/end_to_end_oidc_gating.rs`)**: 8 tests covering
  the full HTTP path — permissive accepts, gated rejects no-token,
  gated accepts valid token, gated rejects bad signature with
  `OIDC_VERIFY_FAILED` code, gated rejects unknown issuer, soft-gated
  accepts no-token but rejects bad token, `/v1/auth/info` surfaces
  configured issuers, `/v1/auth/info` returns empty list on permissive
  relay.

### Deviations / decisions

- **`base64` is now an always-on dep**, not just feature-gated under
  `authority`. The OIDC verifier needs it to peek the JWT payload's
  `iss` claim (we pre-screen the issuer before loading its JWKS so we
  can pick the right allowlist entry). Cost is negligible — `base64`
  was already in the dep tree via `--features authority`.
- **`rsa` as dev-dep only**: tests need RSA keypair generation to mint
  test JWTs and produce JWKS public material. Production never sees
  `rsa` — verification goes through `jsonwebtoken::DecodingKey::from_jwk`
  which is RSA/ECDSA/Ed25519 generic.
- **30s leeway** on `exp`/`nbf` validation. Standard OIDC clock-skew
  tolerance. Adjustable in code if a deployment's clock drift exceeds
  this.
- **Nonce / acr / amr NOT validated**. This is a back-channel
  registration RPC, not an interactive login flow. AAL2 enforcement
  belongs at the IdP, not at the relay verifier.
- **JWKS degraded mode**: if a refresh fails AND a cached (TTL-expired)
  set exists, we reuse the cached set rather than fail closed. Tradeoff:
  preserves availability when an IdP is briefly unreachable, at the cost
  of accepting tokens signed with keys the IdP may have just rotated
  out. The spec addendum documents this trade explicitly.

See `relay/spec/oidc-gating-addendum.md` for the full design + RPC
surface; `relay/deploy/relay.example.toml` for annotated config
examples (permissive / single-issuer / multi-issuer).

## HTTP/3 (in-binary, opt-in)

The relay's REST endpoints (`POST /v1/register`, `POST /v1/heartbeat`,
`POST /v1/deregister`, `GET /health`) are reachable over HTTP/3 when
`ohd-relay serve --http3-listen ADDR:PORT` is set. Implementation lives in
`src/http3.rs`: `quinn 0.11` + `h3 0.0.8` + `h3-quinn 0.0.10`.

### HTTP/3 polish pass (2026-05-08, follow-ups landed)

- **Production cert flags**. `serve` accepts `--http3-cert PATH` /
  `--http3-key PATH` to load a PEM-encoded cert chain + private key from
  disk via `rustls-pemfile 2` (`http3::load_pem_cert_key`). When neither
  flag is set the listener falls back to `dev_self_signed_cert()` and
  emits a stderr warning so production misconfiguration is hard to miss.
- **Streaming `http_body::Body` adapter**. `http3::H3RequestBody`
  implements `Body<Data = Bytes>` and pulls chunks from
  `RequestStream::recv_data` lazily inside `poll_frame`. Mirrors the
  storage server's adapter; the practical win is small here (relay's
  REST endpoints carry small JSON payloads) but parity with the storage
  path keeps the two code shapes uniform and unblocks any future
  large-payload relay endpoints.

The HTTP/2 listener stays bound on the TCP port for clients that prefer it
or that need WebSockets. **WebSocket-over-HTTP/3 is intentionally not
wired**: RFC 9220 (Bootstrapping WebSockets with HTTP/3) requires
extended-CONNECT support that the `h3 0.0.x` crate does not yet expose to
axum cleanly. Tunnel + attach traffic therefore continues to use HTTP/2;
when `h3` ships extended-CONNECT we lift the WS routes onto HTTP/3 too
without changing the axum router shape.

The integration test `tests/end_to_end_http3.rs` covers the QUIC handshake
+ a `GET /health` round-trip end-to-end, pinning the wire so a future
regression that breaks HTTP/3 fails loudly.

## Raw QUIC tunnel (in-binary, opt-in) — landed 2026-05-09

A second QUIC-based listener for the **storage→relay long-lived tunnel**.
Distinct from the HTTP/3 listener above (which carries the REST endpoints
under HTTP/3 framing). When `ohd-relay serve --quic-tunnel-listen ADDR:PORT`
is set, the relay accepts raw QUIC connections on `ADDR:PORT` advertising
ALPN `ohd-tnl1` and dispatches them through `src/quic_tunnel.rs`.

### Why raw QUIC, not WS-over-HTTP/3

The storage→relay tunnel is the primary mobile pain point: phones flip
between WiFi and cellular constantly, and each switch invalidates the
underlying TCP socket the WebSocket-over-HTTP/2 tunnel rides on.
Recovery costs 1–3 s of push-wake + redial + handshake. **Raw QUIC has
native connection migration** (RFC 9000 §9 PATH_CHALLENGE /
PATH_RESPONSE): on a path change the QUIC stack revalidates the new
five-tuple and the connection / streams stay alive. No application-level
reconnect, no push, no stall.

The relay tunnel carries opaque ciphertext (TLS terminates at storage
and consumer; the relay sees DATA-frame ciphertext only), so HTTP
framing on top adds zero value for this surface. RFC 9220 (WebSocket-
over-HTTP/3) is also years away in `h3 0.0.x` — raw QUIC sidesteps that
dependency entirely. This mode isn't a stopgap; it's the right final
design for the storage→relay tunnel.

The **WebSocket-over-HTTP/2 tunnel at `WS /v1/tunnel/:rid` stays wired
as a fallback** for networks that block UDP/443. When both transports
are enabled storage prefers raw QUIC and falls back on UDP-block
detection (the storage-side outbound integration is still TODO; see the
deliverables note below).

### Wire shape

Documented in detail at the top of `src/quic_tunnel.rs`. Summary:

- ALPN: `b"ohd-tnl1"`.
- **Stream 0 (handshake / control)**: client opens first bidi stream,
  sends `[u8 version=0x01][u8 cred_len][cred_len bytes credential]
  [u16 token_len BE][token_len bytes rendezvous_id]`. Relay validates
  against `RegistrationTable`, replies `[ack_status][16 bytes
  session-base-id]`. On reject, relay closes the connection with code
  `REGISTRATION_REJECTED` (1).
- **Per-session streams**: relay opens a fresh bidi stream toward
  storage on each consumer attach, writes
  `[SESSION_OPEN tag=0x01][u32 BE session_id]` then forwards
  `TunnelFrame` envelopes opaquely. Either side `finish()`'s the stream
  to close cleanly; `reset()` for errors.
- **Heartbeats**: control channel `[HEARTBEAT tag=0x02][u64 BE
  timestamp_ms]` every 60 s; 3 misses tear down. QUIC's own PING +
  PATH_CHALLENGE handle network-change keepalive without this; the
  application heartbeat catches dead peers.

### What's wired

- **`src/quic_tunnel.rs`** (NEW). `serve_quic_tunnel(addr, cert, key,
  state, shutdown_rx)` runs the listener; `handle_connection` does the
  handshake, registers a `TunnelEndpoint` in the existing `SessionTable`
  (so the consumer-attach WS handler can route to a QUIC-backed tunnel
  transparently), and spawns the outbound dispatcher + control reader +
  heartbeat pulse.
- **`src/server.rs`** (surgical Edit). `ServeOptions` gains
  `quic_tunnel_listen` / `quic_tunnel_cert` / `quic_tunnel_key`; the
  tunnel listener is spawned alongside the HTTP/2 + HTTP/3 listeners
  with a `tokio::sync::watch` shutdown channel.
- **`src/main.rs`** (surgical Edit). New `--quic-tunnel-listen` /
  `--quic-tunnel-cert` / `--quic-tunnel-key` flags on the existing
  `serve` subcommand.
- **`src/session.rs`** (surgical Edit). Extracted the per-tunnel
  `attached_senders` registry into `pub fn attached_senders_for(...)`
  + `pub type AttachedSenders` so both transports share the same
  storage→consumer routing map. `server.rs`'s `EndpointExt` now
  delegates to it; behaviour is unchanged for the WS path. The map is
  Weak-keyed so dropped endpoints are swept the next time `_for(...)`
  is called.
- **`src/lib.rs`** (surgical Edit). `pub mod quic_tunnel;`.
- **`Cargo.toml`**: no new deps — `quinn 0.11` + `rustls 0.23` + `rcgen
  0.13` were already pulled in by the HTTP/3 work.
- **`deploy/relay.example.toml`** (surgical Edit). Annotated
  `[tunnel.quic]` section explaining listen address, cert/key paths,
  heartbeat interval.
- **`examples/quic_tunnel_client.rs`** (NEW). Minimal `quinn` client
  reference — connects, performs the handshake, accepts session
  streams, echoes DATA frames. Documents the wire for whoever wires
  the storage-side outbound tunnel later.

### Tests (`tests/end_to_end_quic_tunnel.rs`)

Three integration tests, all green and not flaky after 5+ consecutive
runs:

1. **Roundtrip**: spin up the tunnel listener, register a storage,
   dial via `quinn::Endpoint`, perform the handshake, push 4 KiB of
   `TunnelFrame`s in both directions, verify byte-identical delivery.
2. **Migration via `Endpoint::rebind`**: same setup, but after a wave
   of frames flows, rebind the client's underlying UDP socket to a
   fresh ephemeral port (simulating a phone WiFi↔cellular handoff),
   push another wave, verify the connection survives + bytes arrive.
   This is the proof point for the migration claim — quinn handles
   PATH_CHALLENGE / PATH_RESPONSE under the hood and the test asserts
   that's enough on its own (no reconnect, no application-level
   recovery).
3. **Reject**: bad credentials → relay closes with `REGISTRATION_REJECTED`
   (close-code 1).

A subtlety the tests pinned down: `quinn::RecvStream` chunks may carry
multiple `TunnelFrame`s in a single read, so a fresh-buf-per-call
reader silently loses bytes after the first frame. The integration
test uses a buffered helper (`read_one_frame_buffered(recv, &mut buf)`)
that preserves leftover bytes across calls; storage-side
implementations need to do the same.

### Deviations / decisions

- **Separate UDP port** for the tunnel ALPN, not multiplexed onto the
  HTTP/3 endpoint. quinn supports multi-ALPN, but dispatching an
  accepted connection to the right handler post-handshake couples two
  otherwise-independent listeners. Separate ports keep the surfaces
  operationally + code-wise distinct. Documented in
  `quic_tunnel.rs` "ALPN + endpoint isolation".
- **Length-prefixed credential field** (1-byte length + variable bytes,
  capped at 128). The original prompt suggested a 32-byte fixed slot,
  but `generate_credential()` produces ~52-character base32 strings, so
  the fixed slot wouldn't fit. Length-prefixed is also more robust to
  future credential format changes.
- **Watch-channel shutdown** uses `tokio::sync::watch` rather than
  `tokio_util::CancellationToken` to avoid pulling in `tokio-util`
  for a single primitive. The accept loops + per-task shutdown checks
  exit on either `value=true` OR sender-dropped (the latter prevents a
  tight-spin loop when callers forget to send the shutdown signal
  before dropping the sender).
- **Storage-side outbound integration is a separate deliverable.** The
  relay-side listener is fully wired and tested; the storage process
  (`ohd-storage-server`) needs a tunnel-client module that mirrors
  `examples/quic_tunnel_client.rs`'s wire. Per the cross-agent
  coordination rules, that lives in the storage crate.

### Known followups

- **Bandwidth metering / per-session counters** mirror what the WS
  tunnel doesn't have either; same TBD.
- **Cert pinning on the QUIC tunnel cert** is delegated to the operator
  via `--quic-tunnel-cert` / `--quic-tunnel-key`. v1 storage-side
  outbound clients can pin the relay's cert from the registration
  response (see `relay-protocol.md` "Storage registration"). Not a
  relay-side concern.
- **Per-session WINDOW_UPDATE flow control** still uses mpsc-channel
  backpressure as the actual governor. Same status as the WS tunnel.

## Authority mode (feature `authority`)

Build with `cargo build --features authority`. When the binary is built
without the feature, the entire `auth_mode/` subtree is dead-stripped
along with its dep stack.

### What's wired

- **`AuthorityCertChain`** holds leaf PEM + Fulcio intermediate PEM +
  OHD root PEM + the leaf's Ed25519 keypair.
- **`FulcioClient`** speaks Sigstore's standard Fulcio v2 wire:
  `POST /api/v2/signingCert` with the OIDC-bearer + Ed25519
  proof-of-possession body shape from `spec/emergency-trust.md`.
  Returns the parsed cert chain.
- **`RekorClient`** submits an `intoto` v1 log entry per refresh.
  Soft-fail by default — Rekor is auditing, not gating.
- **`AuthorityState`** caches the active chain, `current()` accessor for
  the signing path, `needs_refresh()` for the loop.
- **`run_refresh_loop`** wakes every 60s, refreshes when within ~1h of
  expiry, retries every 5min on failure. Driven as a `tokio::spawn`'d
  task at server startup when `[authority] enabled = true`.
- **`sign_request`** / **`verify_request`** in `auth_mode::signer`:
  - `EmergencyAccessRequest` wire shape (matches
    `spec/emergency-trust.md` field-for-field; `signature` field is the
    base64-encoded Ed25519 sig).
  - **v1 canonical encoding**: SHA-512 of canonical JSON (sorted keys)
    of the request with the signature field stripped. The spec calls
    for "canonical Protobuf encoding" — we don't pull in `prost` for one
    message type in v1; this is documented in the module header. When
    the OHD project ships a shared `prost`-based crate, this swaps out
    in one function.
  - X.509 chain validation: each cert valid at `now`, each child signed
    by parent, chain depth ≤ 4, terminates at one of the
    caller-supplied trust roots (matched by full SPKI), each non-root
    cert carries the OHD emergency-authority EKU OID
    (`1.3.6.1.4.1.99999.1.1` placeholder until IANA assigns OHD's PEN).
- **`POST /v1/emergency/initiate`** HTTP endpoint. Accepts an unsigned
  `EmergencyAccessRequest` shape from an authenticated responder,
  signs it with the cached cert, queues a push-wake to the patient's
  rendezvous, and returns `(signed_request, delivery_status)` where
  status is one of `delivered` / `pushed` / `no_token`.

### What's still hand-wave-y

| What | Status | Path forward |
|---|---|---|
| **OIDC token rotation** | Token file is re-read on every refresh. Rotation is the deployment system's job (cert-manager / sealed-secrets / OIDC sidecar). We do NOT parse the JWT to derive `email_claim`; it's set explicitly in `relay.toml` (`org_label` is reused). | Parse JWT claims server-side in a v1.x pass; auto-extract `email`. |
| **Full RFC 5280 path validation** | We verify chain signatures + validity windows + chain depth + EKU + root SPKI match. We do NOT yet enforce `pathLenConstraint`, name constraints, or basicConstraints `CA:TRUE` on intermediates. Adequate for v1 where the chain shape is fixed; v1.x adds the missing checks. | `auth_mode/signer.rs::verify_request` |
| **Rekor inclusion-proof verification on the verifier side** | Optional per spec, deferred to v1.x. Rekor submission on the writer side IS wired. | `auth_mode/rekor.rs` (writer only); reader side TBD |
| **HSM-backed leaf signing** | Leaf key lives in process memory in v1. Per-process compromise = one short-lived leaf compromised, capped at 24h cert TTL. | Abstract a `SigningHandle` trait, plug PKCS#11 / TPM / cloud-KMS implementations |
| **Responder cert layer (1-4h, per-shift)** | Out of scope for this pass; the relay's leaf is the org cert, not a per-responder cert. | Add `AuthorityState::issue_responder_cert(operator_oidc) -> Result<...>` |
| **`OhdcService.DeliverEmergencyRequest` on storage side** | The relay produces the signed payload; delivering it to the patient's storage flows over the existing tunnel + inner TLS + OHDC RPC layer (storage's job). Per spec. | n/a |
| **iOS Critical Alert in production** | The APNs client supports `ApnsUrgency::Critical`. Apple's Critical Alert entitlement on the OHD Connect bundle ID is Apple-side; applied for through Apple Developer Console under "Capabilities". Without the entitlement Apple silently downgrades the push. | Document in OHD Connect deploy notes; this crate is correct |

## What's stubbed / TBD

| What | Why not done | Where to pick up |
|---|---|---|
| Real FCM HTTP v1 client | ✅ Landed 2026-05-09 in `src/push/fcm.rs`. | n/a |
| Real APNs HTTP/2 + JWT client | ✅ Landed 2026-05-09 in `src/push/apns.rs`. | n/a |
| Authority cert chain + EmergencyAccessRequest signing | ✅ Landed 2026-05-09 under `--features authority`. | n/a |
| HTTP/3 production cert flags | ✅ Landed 2026-05-08 (`--http3-cert PATH` / `--http3-key PATH`). | n/a |
| WebSockets-over-HTTP/3 (RFC 9220) | `h3 0.0.x` doesn't expose extended-CONNECT support cleanly to axum. Tunnel + attach paths stay HTTP/2; `--http3-listen` only fronts the REST surface. | `src/http3.rs` (top-of-file rationale); revisit when h3 ships extended-CONNECT |
| Connect-RPC framing | Plain WebSockets are sufficient for binary frames; switch later if interop needed | `src/server.rs` (handle_tunnel_ws / handle_attach_ws) |
| `RefreshRegistration` RPC | Not in the priority list; trivial to add | New `handle_refresh` in `src/server.rs` |
| `POST /v1/pair` (pairing-mediated) | Lower priority; pairing table exists | Wire to `src/pairing.rs::PairingTable::insert` |
| Per-session flow control via `WINDOW_UPDATE` | mpsc channel buffers act as backpressure for now; spec asks for explicit 256 KB windows | `src/server.rs::handle_storage_frame` |
| Bandwidth metering / per-session counters | Not built | Stub fields exist in `state.rs` |
| Operator setup UI / one-time registration tokens | Per-OIDC gating landed 2026-05-09 (`[auth.registration]` block); covers OHD Cloud + clinic-SSO deployments. The legacy "one-time code from setup web UI" pattern stays available for self-hosted-without-OIDC operators (permissive default). | n/a |
| Multi-instance sharding | v1 single-instance | `RegistrationTable::lookup_by_rendezvous` is the natural shard key |
| Tracing JSON output / structured ops logs | Just human-readable for now | `init_tracing` in `src/main.rs` |
| `tokio_tungstenite` 0.21 vs newer | We pull in 0.21 in dev only; library uses axum's WS | n/a |

## Production deployment notes (push secrets + OIDC)

The relay never reads secrets out of `relay.toml` directly — it reads
filesystem paths the deployment system mounts:

- **Docker / Compose**: use [Docker
  secrets](https://docs.docker.com/engine/swarm/secrets/) (Swarm mode) or
  bind-mount a tmpfs volume from a sealed-source. Reference the path in
  `relay.toml`'s `service_account_path` / `key_path` /
  `oidc_id_token_path`.
- **Kubernetes**: mount as `Secret` volumes; pair with
  [sealed-secrets](https://github.com/bitnami-labs/sealed-secrets) or
  [external-secrets](https://external-secrets.io/) so the Helm chart
  doesn't carry plaintext. The relay's pod reads them as files; no env
  var indirection.
- **systemd**: `LoadCredential=` keeps secrets in a tmpfs with
  per-service ACLs; reference `${CREDENTIALS_DIRECTORY}/...` in
  `relay.toml`.
- **OIDC ID-token rotation (authority mode)**: the relay re-reads
  `oidc_id_token_path` on every refresh (every ~23h once steady-state).
  Keep token TTL longer than the refresh window (≥1h headroom) so a
  rotation race doesn't cause a refresh failure.
- **APNs `.p8` rotation**: APNs auth keys are long-lived (created in
  Apple Developer Console). Rotate by issuing a new key, mounting it,
  bouncing the relay process. The first push after restart re-mints the
  ES256 JWT.
- **FCM service-account rotation**: `gcloud iam service-accounts keys
  create` → mount → bounce. The cached OAuth2 access token is
  invalidated on bounce; the next push refreshes it.

## Deviations from the prompt

- **Endianness clarification**: prompt mentioned "little-endian where the
  spec specifies." The actual `relay-protocol.md` calls all multi-byte
  fields **big-endian**. I followed the spec, since "spec wins when
  prompt and spec conflict" is the documented rule in `SPEC.md`. The
  codec is big-endian throughout.
- **Endpoint paths**: I used `/v1/register`, `/v1/heartbeat`,
  `/v1/deregister`, `/v1/tunnel/:rid`, `/v1/attach/:rid`. The wire spec
  says `/relay/v1/...`; I chose the shorter prefix matching the prompt's
  deliverables list. Either prefix is fine; renaming is a one-line Caddy
  rewrite.
- **`WS /v1/attach/:rid`**: spec says "consumer attach at
  `https://<host>/r/<rendezvous_id>`". I used the WS path because we
  speak frames over WebSockets, not Connect-RPC. When swapping to
  Connect-RPC, restore the `/r/...` path.
- **`session.rs::SessionRow` was renamed/reshaped** into `TunnelEndpoint`
  + per-endpoint sender map + a separate `attached_senders` registry
  threaded into the endpoint via a small extension trait. The state-of-
  the-world is still the same five-field per-user invariant; only the
  in-memory plumbing differs from the original scaffold.
- **`SessionRelaySide::take_consumer_to_storage`** is currently unused —
  the consumer→storage path threads bytes directly via WS read → frame
  decode → tunnel `outbound_tx`. Left as dead code path because future
  revisions (especially LAN fast-path) may want to drive it through the
  channel instead of the WS reader directly.

## Files added since scaffolding

```
relay/
├── src/
│   ├── frame.rs           (binary frame codec + tests)
│   ├── pairing.rs         (pairing table + TTL sweeper)
│   ├── session.rs         (tunnel + session multiplexing)
│   ├── lib.rs             (library entry for integration tests)
│   ├── state.rs           (SQLite-backed RegistrationTable)
│   ├── server.rs          (axum HTTP + WS endpoints + emergency endpoint)
│   ├── http3.rs           (in-binary HTTP/3 listener for REST endpoints)
│   ├── main.rs            (clap CLI: --config, --db, --port, --bind, --http3-*)
│   ├── config.rs          (NEW 2026-05-09 — relay.toml loader; +[auth.registration] 2026-05-09)
│   ├── auth/
│   │   ├── mod.rs         (NEW 2026-05-09 — re-exports OidcVerifier)
│   │   └── oidc.rs        (NEW 2026-05-09 — JWKS-cache-backed id_token verifier)
│   ├── push/
│   │   ├── mod.rs         (NEW 2026-05-09 — replaces push.rs; trait + dispatcher)
│   │   ├── fcm.rs         (NEW 2026-05-09 — real FCM HTTP v1 client)
│   │   └── apns.rs        (NEW 2026-05-09 — real APNs HTTP/2 client)
│   ├── auth_mode/
│   │   ├── mod.rs         (NEW 2026-05-09 — replaces auth_mode.rs; feature-gated)
│   │   ├── cert_chain.rs  (NEW 2026-05-09 — AuthorityCertChain + leaf parsing)
│   │   ├── fulcio.rs      (NEW 2026-05-09 — Sigstore Fulcio v2 signingCert client)
│   │   ├── rekor.rs       (NEW 2026-05-09 — minimal Rekor v1 intoto submitter)
│   │   ├── refresh.rs     (NEW 2026-05-09 — AuthorityState + run_refresh_loop)
│   │   └── signer.rs      (NEW 2026-05-09 — sign_request / verify_request)
│   └── quic_tunnel.rs     (NEW 2026-05-09 — raw QUIC tunnel listener, ALPN ohd-tnl1)
├── tests/
│   ├── end_to_end.rs              (4 integration tests for tunnel + attach)
│   ├── end_to_end_http3.rs        (1 HTTP/3 smoke test)
│   ├── end_to_end_emergency.rs    (NEW 2026-05-09 — 4 sign/verify integration tests, gated on `--features authority`)
│   ├── end_to_end_oidc_gating.rs  (NEW 2026-05-09 — 8 OIDC-gated registration tests against a mock IdP)
│   └── end_to_end_quic_tunnel.rs  (NEW 2026-05-09 — 3 raw QUIC tunnel tests: roundtrip, migration via Endpoint::rebind, reject-on-bad-creds)
├── examples/
│   └── quic_tunnel_client.rs      (NEW 2026-05-09 — minimal quinn client demonstrating the wire shape; reference for storage-side outbound integration)
├── deploy/
│   └── relay.example.toml         (REWRITTEN 2026-05-09 — new push.fcm / push.apns / authority sections; +[tunnel.quic] 2026-05-09)
├── Cargo.toml                     (UPDATED 2026-05-09 — reqwest+rustls, jsonwebtoken, toml, x509-parser, ed25519-dalek, pem, base64)
└── Cargo.lock                     (UPDATED — frozen dep graph)
```

### Files removed (replaced by directory modules)

- `src/push.rs` → `src/push/{mod,fcm,apns}.rs`
- `src/auth_mode.rs` → `src/auth_mode/{mod,cert_chain,fulcio,rekor,refresh,signer}.rs`

## Test strategy used

- **Unit**: frame round-trip + every malformed-frame path; SQLite
  registration ops + recent-events log; pairing TTL + sweeper (uses
  `tokio::time::pause` + `tokio::time::Instant`); push routing by
  platform; FCM payload shape + URL substitution + service-account
  parser + cached-token freshness; APNs envelope shape + JWT cache
  freshness + UUID format; config TOML round-trip; cert chain validity
  windows + wire ordering; signer canonical bytes + sign-then-verify
  + the four common rejection paths (bad sig / unknown root / depth
  exceeded / pin mismatch).
- **Integration**: end-to-end 4 KiB roundtrip test; deregister-200; bad
  credential-401; unknown-rendezvous-on-attach-error;
  HTTP/3 health round-trip; under `--features authority`, four
  emergency sign/verify integration tests hitting the public
  `auth_mode::sign_request` / `verify_request` API end-to-end with
  `rcgen`-minted self-signed Ed25519 chains.
- **Smoke**: `cargo run -- serve` boots, listens, logs. With
  `--features authority` and `[authority] enabled = true` plus a
  reachable Fulcio, the refresh loop logs the issued cert.

## Cross-references

- Component spec: `../spec/docs/components/relay.md`
- Wire spec: `./spec/relay-protocol.md`
- Authority cert mechanism: `./spec/emergency-trust.md`
- Push payload contract: `./spec/notifications.md`
- Per-OIDC gating addendum: `./spec/oidc-gating-addendum.md`
- This crate's local spec: `./SPEC.md`
