# Design: OHD Relay Wire Protocol

> The bit-level spec for the Relay tunnel: cert/identity model, tunnel framing, multiplexing, lifecycle. The wire-level companion to [`../components/relay.md`](../components/relay.md) (which describes Relay's purpose, operator landscape, and persistence model).

## What this doc pins down

The thing the rest of the spec has been waving at: **how a consumer's TLS handshake reaches a phone-hosted storage through an opaque relay, when neither end has a CA-issued cert for the relay's domain.**

Once this is fixed, the relay becomes what it claims to be: an opaque tunnel that sees only ciphertext, can be DoS'd but not eavesdropped on, and works regardless of which jurisdiction or operator runs it.

## TLS-through-tunnel: the cert / identity model

### The problem

A consumer (e.g. OHD Care) opens an HTTP/3 connection to `https://relay.example.com/r/<rendezvous_id>`. The relay forwards bytes to a storage instance — typically a phone or a NAS, which doesn't have a CA-issued cert and can't get one (no public DNS name, no Let's Encrypt path).

Naïve options that don't work:

- **TLS terminates at the relay** — relay sees plaintext. Defeats the privacy property; subpoena-readable.
- **CA-issued certs for every storage** — would require every phone to obtain a Let's Encrypt cert under some domain, manage renewals, etc. Operationally untenable.
- **mTLS with a custom CA** — pushes CA management onto the user; the wrong direction.

### The solution: storage identity key + cert pinning via grant artifact

Each OHD Storage instance has a long-lived **identity key** (Ed25519, generated on first launch — already specified in [`encryption.md`](encryption.md) "Identity key"). The storage uses this key to:

1. Generate a **self-signed TLS certificate**. SAN = its rendezvous URL (`relay.example.com/r/<rendezvous_id>`). Validity 90 days; renewed automatically by the storage, signed by the same identity key.
2. Publish the cert's **public-key fingerprint** (SHA-256 of the SubjectPublicKeyInfo) in:
   - Every grant artifact the user issues (`ohd://grant/...?pin=<base64url(sha256_spki)>`)
   - The `OhdcService.WhoAmI` and `OhdcService.Health` responses (so consumers can verify after connecting)
   - The storage's relay registration (so the relay can reject obviously-mismatched re-registrations)

When a consumer (Care, Connect, MCP) wants to talk to the storage:

1. Consumer parses the grant artifact: extracts `(token, rendezvous_url, pin)`.
2. Consumer opens an HTTP/3 connection to the rendezvous URL.
3. The Relay accepts the connection. It does **not** terminate TLS — instead it negotiates the OHDC tunnel framing (next section) and starts forwarding bytes to the registered storage.
4. **Inside that forwarded byte stream, the consumer and storage do their own TLS 1.3 handshake.** The consumer offers TLS, the storage presents its self-signed cert (signed by identity key), the consumer verifies the cert's SPKI fingerprint against the pin from the grant.
5. Mismatch → consumer aborts with a clear "this storage isn't who the grant said it would be" error. Likely either: the user changed their identity key (re-issue grants), or someone has intercepted the relay (more concerning).
6. Match → TLS session established end-to-end through the relay. From here on, OHDC operations flow over this TLS-inside-tunnel session. The relay sees encrypted bytes only.

### Why this works

- **The relay can't read traffic** — it sees the outer ciphertext (consumer↔relay, on `relay.example.com`'s cert) and the inner ciphertext (consumer↔storage, on the storage's self-signed cert). The inner is what carries OHDC; the outer just authenticates the relay-mediated path.
- **The consumer doesn't need to trust the relay** — even a malicious relay can't decrypt or modify OHDC traffic without breaking TLS 1.3, because the inner TLS session terminates at storage, not at the relay.
- **The consumer doesn't need a CA** — the pin in the grant artifact is the trust anchor. The user authorized the grant; the user knows their storage's identity key; the pin proves that.
- **Identity-key rotation is rare and explicit** — when it happens, the user re-issues outstanding grants with the new pin (per [`encryption.md`](encryption.md) "Identity key"). Old grants stop working, which is the correct behavior.
- **Cert renewal is invisible** — the storage rotates its 90-day TLS cert without changing the identity key, so the SPKI fingerprint stays the same; consumers' pins stay valid.

### Pin format in grant artifacts

The grant share URL grew an explicit `pin` parameter:

```
ohd://grant/<token>?storage=<rendezvous_url>&pin=<sha256_spki_base64url>[&case=<case_ulid>]
```

Where:
- `<token>` is the `ohdg_…` (or `ohdd_…` for device-bound) credential.
- `<rendezvous_url>` is the relay's public URL for this storage (or the storage's direct URL if directly reachable).
- `<sha256_spki_base64url>` is the storage's identity key fingerprint, base64url-encoded.
- `<case_ulid>` (optional) is set when the grant is case-bound.

The QR-coded form encodes the same URL. Connect mobile presents the URL and the QR; the grantee pastes / scans either.

### The directly-reachable case (no relay)

When the storage is directly reachable (cloud / custom-provider deployment), the same pin mechanism applies *or* the storage uses a CA-issued cert:

- **Cloud / custom-provider**: typically uses a CA-issued cert (Let's Encrypt via Caddy on the operator's public domain). The grant artifact's `pin` parameter is omitted — TLS validates against the public CA chain like any HTTPS site.
- **Self-hosted at a public URL**: same as cloud.
- **Self-hosted behind NAT** (going through a relay): pin used.
- **On-device** (always going through a relay): pin used.

Consumers that find the `pin` parameter present **must** validate against it (and fail closed on mismatch, even if a CA chain would otherwise validate). Consumers that find no `pin` validate against system CAs as normal HTTPS.

### Identity key rotation

When the user rotates their storage's identity key (per [`encryption.md`](encryption.md) "Key rotation" / "Identity key"):

1. Storage generates new keypair, regenerates its self-signed cert.
2. All outstanding grants that carry the old pin become invalid — consumers fail with `CERT_PIN_MISMATCH` on next connection.
3. User receives a notification: "Your active grants need to be re-issued because you rotated your storage's identity key."
4. User opens Connect → "Active grants" → "Re-issue all" or per-grant "Re-issue with new artifact." Generates fresh share URLs/QRs containing the new pin.
5. User redistributes the new artifacts to grantees (text, in-person QR, etc.).

This is rare but loud. The disclosure UX is essential — the user must understand they're invalidating ongoing access; if it's not actually a security event, they shouldn't rotate.

## Tunnel framing

The transport between storage and relay (tunnel registration) and between consumer and relay (consumer attach) is HTTP/3. Inside each connection, OHDC traffic flows as **typed binary frames**.

### Why frames (not raw byte forwarding)

The relay needs to demultiplex many consumer connections onto one storage tunnel and route each consumer's bytes to a session that the storage can identify. Raw byte forwarding can't do that without further connections per consumer, which doesn't work for a single long-lived storage tunnel. So: framing.

### Frame format

Every tunnel frame is binary, big-endian:

```
0       1       2       3       4
+-------+-------+-------+-------+
| MAGIC | TYPE  | FLAGS | RSVD  |
+-------+-------+-------+-------+
| SESSION_ID (4 bytes)          |
+-------+-------+-------+-------+
| PAYLOAD_LEN (4 bytes, BE u32) |
+-------+-------+-------+-------+
| PAYLOAD (PAYLOAD_LEN bytes)   |
+-------+-------+-------+-------+
```

| Field | Bytes | Meaning |
|---|---|---|
| MAGIC | 1 | `0x4F` ('O' for OHD). Sanity / framing-resync byte. |
| TYPE | 1 | Frame type (table below). |
| FLAGS | 1 | Frame-type-specific flag bits; reserved bits MUST be zero. |
| RSVD | 1 | Reserved for future use; MUST be zero. |
| SESSION_ID | 4 | Per-consumer-session identifier assigned by the relay; zero for control frames not bound to a session. |
| PAYLOAD_LEN | 4 | Big-endian uint32; payload byte length. Max 65535 — payloads larger split into multiple DATA frames. |
| PAYLOAD | varies | Frame-type-specific. |

### Frame types

| TYPE | Name | Direction | Payload |
|---|---|---|---|
| `0x01` | `HELLO` | Consumer↔Relay, Storage↔Relay | Handshake; capability negotiation. |
| `0x02` | `OPEN` | Relay→Storage | Notifies storage that a new consumer attached, with a fresh session_id. Payload includes consumer's claimed grant token (so storage can reject before incurring TLS cost). |
| `0x03` | `OPEN_ACK` | Storage→Relay | Storage accepts the open; ready for DATA frames. |
| `0x04` | `OPEN_NACK` | Storage→Relay | Storage rejects (e.g. `INVALID_TOKEN`, `RATE_LIMITED`). Relay translates to a clean close on the consumer side. |
| `0x05` | `DATA` | Bidirectional | Opaque ciphertext bytes (TLS records, after the inner TLS handshake completes). |
| `0x06` | `CLOSE` | Bidirectional | Tear down the session identified by SESSION_ID. Payload optionally carries a reason code. |
| `0x07` | `PING` | Bidirectional | Keepalive; correlated with PONG. |
| `0x08` | `PONG` | Bidirectional | Reply to PING. |
| `0x09` | `WAKE_REQUEST` | Relay→Storage (out-of-band; via push) | Special: not a tunnel frame; it's a push notification triggering the storage's mobile app to re-establish the tunnel. Mentioned here for completeness. |
| `0x0A` | `WINDOW_UPDATE` | Bidirectional | Flow-control hint; informs the peer how much more payload it's prepared to accept on this session. |
| `0x80`..`0xFF` | Reserved | — | For vendor / experimental extensions; must not collide with future spec. |

The wire format is intentionally close to a tiny custom binary protocol (similar in spirit to QUIC frames or HTTP/2 frames). Implementing it is a few hundred lines in any language; the tunnel doesn't need a full HTTP stack.

### Session lifecycle

Multi-consumer multiplexing works via SESSION_ID:

1. **Storage tunnel established**: storage opens an HTTP/3 connection to the relay; the connection itself authenticates via the storage's registration credential (see "Storage registration" below). Storage and relay exchange `HELLO` frames (capability bits, supported frame types, max payload size).
2. **Consumer arrives**: consumer opens an HTTPS connection to the relay's `https://relay.example.com/r/<rendezvous_id>` URL. Consumer and relay exchange `HELLO` frames. Consumer presents its grant token in the HELLO payload (so relay can reject obviously-bad tokens before even forwarding to storage).
3. **Relay assigns SESSION_ID**: relay picks an unused 32-bit id for this consumer-session. Sends `OPEN` to storage (with the session id and the consumer-supplied token preview).
4. **Storage decides**: storage validates the token (real auth check; `OPEN` is just the relay's hint). On accept, storage sends `OPEN_ACK` and is ready for DATA. On reject, `OPEN_NACK` with an error code.
5. **TLS handshake**: consumer initiates inner TLS. Bytes flow as DATA frames (consumer→storage and storage→consumer), tagged with the same SESSION_ID. Relay just forwards.
6. **OHDC traffic**: once inner TLS is up, OHDC RPCs ride the same DATA frames.
7. **Session close**: either side sends `CLOSE` for that SESSION_ID. The other side cleans up. Other sessions on the same storage tunnel are unaffected.
8. **Storage tunnel close**: storage drops the connection (clean shutdown or unexpected). All sessions die. Relay sets the registration's `current_status='reconnecting'` and waits.

### Flow control

Each session has independent flow-control windows. Default: 256 KB receive window per side; updated by `WINDOW_UPDATE` frames as the receiver consumes bytes.

This is per-session, not per-tunnel — so one slow consumer can't starve others.

### Frame size

Max payload per frame: 65535 bytes. Larger blocks (sample blocks, attachment chunks) split into multiple DATA frames at the application layer (Connect-RPC streams handle this transparently above TLS).

## Storage registration

When a storage first registers with a relay (the durable grant-mediated pattern from [`../components/relay.md`](../components/relay.md) "Grant-mediated"):

```
Storage → Relay: POST /relay/v1/register
                Authorization: Bearer <one-time registration token>
                Body (Connect-RPC RelayService.Register):
                  {
                    storage_pubkey: <Ed25519 SPKI bytes>,
                    storage_pubkey_signature: <Ed25519 sig over registration nonce>,
                    push_token: <optional FCM/APNs token>,
                    user_label: <opaque short label, optional>
                  }

Relay → Storage:  201 Created
                  Body (RegisterResponse):
                  {
                    rendezvous_id: <opaque, e.g. 22-char base32>,
                    rendezvous_url: "https://<relay-host>/r/<rendezvous_id>",
                    relay_pubkey: <Ed25519 SPKI bytes for the relay's identity>,
                    long_lived_credential: <opaque token used for subsequent tunnel auth>
                  }
```

The "one-time registration token" is obtained out of band — typically the user pastes a relay URL into Connect, Connect fetches a one-time registration code from the relay's web UI (`https://relay.example.com/setup`), and uses that code as the bearer.

Subsequent connections (tunnel re-establishment after a phone wakes from sleep, etc.) use the `long_lived_credential` returned at registration. This credential authenticates the storage to the relay; it does **not** authenticate end-users or consumers. It's purely a "yes, I'm the storage that registered as rendezvous_id X."

### `OpenTunnel` (from RelayService)

Once registered, the storage opens its long-lived HTTP/3 connection:

```
Storage → Relay: POST /relay/v1/tunnel
                 Authorization: Bearer <long_lived_credential>
                 Connection: Upgrade-equivalent for Connect-RPC bidi-streaming
                 Body: stream of TunnelFrame (HELLO, then DATA/PING/...)
Relay → Storage: stream of TunnelFrame (OPEN, DATA, CLOSE, PING, ...)
```

This is the `RelayService.OpenTunnel` RPC sketched in [`ohdc-protocol.md`](ohdc-protocol.md). The frames are the binary `TunnelFrame` shape from "Frame format" above; Connect-RPC carries them as raw byte chunks within the bidirectional stream.

### Refresh / heartbeat / deregister

```
RelayService.RefreshRegistration(push_token, label) — updates push token & label without re-registering
RelayService.Heartbeat() — silent keepalive at the registration level (separate from PING frames inside the tunnel)
RelayService.Deregister() — drops the registration; relay removes the rendezvous record
```

Heartbeats every 25 seconds at the tunnel level (PING frame). Deregistration is a clean farewell; the relay also drops registrations after 30 days of no successful tunnel-open as garbage collection.

## Pairing-mediated pattern

For the in-person handshake case (NFC/QR pairing for short doctor-at-desk sessions), see [`../components/relay.md`](../components/relay.md) "Pairing-mediated." The wire shape is the same as grant-mediated — consumer arrives, relay assigns SESSION_ID, frames flow — but the trust anchor is the pairing nonce instead of the grant token. Specifically:

1. Phone and operator's device exchange the pairing nonce out-of-band (NFC tap or QR).
2. Phone opens a short-lived "pairing" registration with the relay: `POST /relay/v1/pair` with the pairing nonce. Relay returns a one-shot rendezvous URL and a per-pairing credential.
3. Operator's client connects to the rendezvous URL, presents the same pairing nonce.
4. Frames flow as in grant-mediated.
5. Pairing credential expires when the session ends or after 30 minutes of inactivity.

The cert pin in this case comes from the NFC/QR payload, not from a long-lived grant.

## LAN fast-path

When consumer and storage discover they're on the same LAN (mDNS for `_ohd._tcp.local`, or relay-mediated STUN-style hint), they migrate the session off the relay onto a direct LAN connection:

1. During an active session, both sides scan for the peer. mDNS first (consumer Wi-Fi); fallback to a relay-mediated UDP-hole-punch if mDNS is blocked (enterprise networks).
2. On discovery: storage publishes a service record `_ohd._tcp.local` advertising its LAN IP + port. Consumer connects directly.
3. Consumer initiates an inner TLS handshake using the same cert pin (the storage uses the same self-signed cert as on the relay path).
4. On successful TLS: in-flight OHDC operations migrate to the new LAN connection (Connect-RPC's stream multiplexing handles this transparently if implemented; otherwise the consumer drops in-flight requests and retries on the LAN connection).
5. Both sides send `CLOSE` on the relay-side session. Tunnel stays alive for other consumers.

LAN fast-path is opt-in per deployment (some clinics' networks don't permit mDNS / UDP broadcasts). Fallback is staying on the relay forever, which works fine.

## Bandwidth metering and rate limiting

Per [`../components/relay.md`](../components/relay.md) "Auth and accounting":

- Per-rendezvous (per-user-storage) byte counters on the relay.
- Per-session byte counters.
- Per-user / per-pairing rate limits — relay rejects new sessions when exceeded, with `429 RATE_LIMITED` returned to the consumer.

These metrics live alongside the registration table; metering is operational telemetry, not part of OHDC.

## Trust model recap

This wire spec doesn't change the trust model in [`../components/relay.md`](../components/relay.md) "Trust model"; it implements it:

| Threat | Defense (how this spec implements it) |
|---|---|
| Passive eavesdropping at the relay | Inner TLS 1.3 (storage cert pinned via grant); relay sees only DATA frame ciphertext. |
| Malicious relay operator | Same. Relay can DoS but cannot read or forge. The cert pin proves the storage's identity. |
| Forged session attach | Pairing nonces are short-lived; grant tokens are validated by storage on `OPEN` (relay can pre-screen but storage is final). |
| Consumer presenting a stolen grant | Storage validates the token via `OhdcService.WhoAmI`-equivalent on inner TLS handshake; revoked tokens rejected. |
| Compromised storage endpoint | Outside relay's scope; OHDC tokens and grants live on the storage device regardless. |
| Subpoena / legal compulsion at relay | Relay can be compelled to log connection metadata (rendezvous id, session counts, byte volumes, push tokens). Cannot disclose payloads. Cert pin protects against MITM-by-court-order. |

## Implementation effort

The tunnel layer is about ~500 lines of Rust (frame parser/serializer, session table, dispatch loop). The relay binary as a whole is ~2k lines including OAuth-proxy-style metadata endpoints, the registration HTTP handlers, metering, and operational telemetry.

The TLS-through-tunnel piece is mostly client-side: storage embeds a `rustls` server with the self-signed cert; consumer uses `rustls` with a custom verifier that checks the SPKI fingerprint against the pin. The relay never touches TLS — it just forwards DATA frames.

## Cross-references

- Relay component overview, persistence, multi-consumer multiplexing: [`../components/relay.md`](../components/relay.md)
- OHDC protocol (what flows over the tunnel after TLS): [`ohdc-protocol.md`](ohdc-protocol.md)
- Identity key + cert renewal mechanism: [`encryption.md`](encryption.md)
- Grant artifact format (what the user shares with grantees): [`auth.md`](auth.md), [`care-auth.md`](care-auth.md)
- Notification system (push-wake when phone storage is asleep): [`notifications.md`](notifications.md)

## Open items (forwarded)

- **Cert pinning vs. trust-on-first-use** for non-grant-bound consumers. v1 requires the pin to be present in the grant artifact. A future "discoverable storage" mode (no pre-issued grant) would need TOFU + clear UX. Not v1.
- **mDNS hint forwarding through the relay** for LAN fast-path on networks that block native mDNS. Mentioned in [`../components/relay.md`](../components/relay.md) Open items; mechanism not specced here.
- **Relay federation** — one user registered with multiple relays simultaneously, with consumers auto-failing-over. v1 picks one; future revision may add forwarding pointers.
- **Per-grant sub-rendezvous** for compartmentalization — each grant gets its own rendezvous URL so the user can revoke a relay-side path without invalidating the storage's main rendezvous. Operationally heavier; deferred.
