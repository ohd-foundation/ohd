# Component: OHD Relay

> The bridging service that lets remote OHDC clients reach an OHD Storage instance that can't accept inbound connections. Forwards opaque packets between paired endpoints; does not decrypt, does not authenticate request payloads.

## Purpose

OHD Relay solves a structural problem: many OHD Storage deployments are not directly reachable from the public internet, yet authorized clients (OHD Care, OHD Connect on a different device, OHDC integrations) need to reach them.

The two main cases:

- **On-device storage**: the user's storage lives on their phone. Phones are behind cellular NAT, on metered radios, sleeping aggressively. No public IP, no inbound connectivity.
- **Self-hosted storage on a home network**: NAS, Pi, home server behind residential NAT or carrier-grade NAT, possibly without static IP, often without port forwarding configured. No public IP either.

Both cases produce the same shape: storage connects out, client connects in, Relay forwards bytes between them. TLS terminates at storage and at client; Relay sees ciphertext only.

This is **not** an "edge case for paranoid users." For any user who picks on-device or home-self-hosted with no public IP, every external query flows through Relay. It's on the critical path of those deployment topologies.

## What Relay does

- **Connection rendezvous**. Provides a meeting point where storage and clients find each other — either via a short-lived pairing nonce (in-person handshake) or via a persistent registration tied to a grant.
- **Opaque packet relay**. Forwards bytes between the two endpoints. TLS is end-to-end; Relay sees ciphertext.
- **Connection authentication**. The pairing or registration handshake establishes a session-bound credential that both endpoints present. Relay rejects connections not part of an active pairing or registration.
- **Session lifecycle management**. Holds sessions open while at least one endpoint is connected; cleans up on idle / explicit close / endpoint disconnect.
- **Bandwidth metering and rate limiting**. Operators want to know who's using the relay and prevent abuse.
- **LAN fast-path negotiation**. Helps endpoints discover when they're on the same LAN; once detected, the active session migrates off Relay onto a direct LAN link with the same end-to-end TLS.

## What Relay does not do

- Relay does not authenticate the OHDC request payload. Storage's OHDC interface still validates the token (self-session / grant / device) on every request; Relay doesn't see the token (it's inside the TLS tunnel). A leaked Relay session credential gets you a forwarding tube, not access to data.
- Relay does not decrypt. A compromised Relay cannot read the user's data. It can deny service, observe traffic patterns, and (with cooperation from a malicious endpoint) splice a session — all of which the audit log surfaces because the storage and client endpoints log what *they* see independently.
- Relay does not store data. Sessions are ephemeral; logs are operational telemetry, not health data.

## Two routing patterns

### Pairing-mediated (ephemeral, in-person)

Used when storage is on a phone, the session is short, and the trust anchor is physical proximity. Typical case: doctor and patient at a desk.

1. **In-person handshake**. Patient taps phone to operator's device (NFC) or operator scans a QR on the patient's phone. The handshake exchanges:
   - The Relay URL the patient has configured (project-run, clinic-run, or self-hosted).
   - A short-lived pairing nonce.
   - The grant the patient is offering for this session (or a reference to one the patient has pre-authorized).
2. **Phone connects to Relay**. Phone opens an HTTP/3 session to its configured Relay, presents the pairing nonce, gets allocated a session id.
3. **Operator's client connects to Relay**. Operator's OHD Care (or other OHDC client) opens an HTTP/3 session to the same Relay, presents the pairing nonce, attaches to the session.
4. **TLS handshake end-to-end**. Phone and client complete a TLS handshake through the relay. From this point on, Relay sees only opaque bytes.
5. **OHDC operations begin**. Client calls `query_events(...)`, `submit_clinical_note(...)`, etc., with the patient's grant token. Storage on the phone resolves and returns. Audit log on the phone records each access.
6. **LAN probe**. In parallel, both endpoints attempt LAN discovery (mDNS for `_ohd._tcp` on the local network, well-known link-local addresses, or a passive listener). On success, the session migrates off Relay to a direct LAN connection — same TLS, lower latency, no internet hop.
7. **Session ends**. The patient closes manually, or the phone goes offline / sleeps too long. The operator's last-fetched data becomes their snapshot, owned under their HIPAA/GDPR posture.

### Grant-mediated (durable, remote)

Used when storage is on a home server (always-on, no public IP) or when a phone wants to be reachable for ongoing remote access. The trust anchor is the user's grant token; physical proximity is not required.

1. **Storage registers with the Relay** (one-time, at user setup or first launch). The storage opens an HTTP/3 connection to the user's configured Relay and registers; the Relay assigns an opaque **rendezvous ID** and returns the public **rendezvous URL** `https://<relay-host>/r/<rendezvous_id>`. This URL is stable for the lifetime of the registration; every grant the user later issues against this storage embeds the same URL.
2. **User issues a grant** via OHD Connect. The grant artifact (the bytes the user shares with the grantee) bundles `(token, rendezvous_url, storage_cert_pin)`.
3. **Storage maintains the tunnel**. Heartbeats keep it alive; for phones, push-notification-triggered wake brings the tunnel back when consumers attempt to attach. See "Persistence" below.
4. **Client connects via the rendezvous URL**. The grantee's client (OHD Care, etc.) opens HTTP/3 to the rendezvous URL, presents the grant token (inside its own TLS handshake to the storage, end-to-end), gets routed to the matching storage tunnel.
5. **TLS handshake end-to-end**. As above; Relay forwards bytes only.
6. **OHDC operations**. As above.
7. **LAN fast-path** if applicable.

The two patterns coexist on the same Relay deployment; storage / client decide which to use based on the grant's metadata.

## Registration model

The grant-mediated pattern is the workhorse of the Relay; this section pins its invariants.

### One user, one rendezvous per relay

A storage instance registers with a relay **once per user**. The Relay assigns a single stable opaque rendezvous ID; that ID maps internally to the user's tunnel. Every grant the user issues against that storage references the same rendezvous URL.

There is no separate "storage identity" in OHDC — `user_ulid` is the only stable wire identity, and a user has at most one canonical primary storage at any moment (per [`../design/storage-format.md`](../design/storage-format.md) "Deployment modes and sync"). Whatever instance is currently primary is what the relay routes to. Migration between deployment modes (phone → cloud → self-hosted) keeps the same `user_ulid`; the new storage re-registers with the relay; existing grants either continue to work (rendezvous ID preserved on a planned migration) or are re-issued by the user (rendezvous ID changes).

The rendezvous ID is **opaque random**, not derived from `user_ulid`. The Relay alone holds the mapping. This avoids fingerprinting: a grantee who sees the rendezvous URL learns nothing about the user's identity.

Switching to a different relay (project-run to clinic-run, etc.) is a fresh registration → new opaque ID → grants get re-issued with the new URL. **Multi-relay-simultaneous** (one user registered with two different relays at the same time) is not supported in v1; the user picks one. Future revisions can add forwarding pointers if real demand surfaces.

### Multi-consumer multiplexing

Many consumers attach via the same rendezvous URL — Care for Dr. A, Care for Dr. B, the user's MCP on a laptop, Connect web on the user's tablet, a sensor backend, a family member's app — and all multiplex onto the **single tunnel** between the storage and the relay. Each consumer presents its own grant, device, or self-session token; the storage demuxes by token after the bytes arrive. The relay sees only ciphertext frames and forwards them to the registered tunnel.

This is the property that makes "one relay URL, all my grants reference it" work cleanly. The user doesn't manage rendezvous-per-grantee; the relay doesn't need to.

### Registration state

The Relay's per-user state is exactly five fields:

```
(rendezvous_id, user_ulid, current_tunnel_endpoint, push_token, last_heartbeat_at)
```

Plus a small log of recent connection events for operational telemetry. **Nothing else** — no grants, no audit, no tokens, no PII beyond what the operator chose to log. The Relay's privacy property — "compromise reveals only traffic patterns" — depends on this minimalism.

Storage-side state lives in the deployment's system DB:

```sql
CREATE TABLE storage_relay_registrations (
  id                       INTEGER PRIMARY KEY AUTOINCREMENT,
  relay_url                TEXT NOT NULL,
  rendezvous_id            TEXT NOT NULL,
  rendezvous_url           TEXT NOT NULL,
  registered_at_ms         INTEGER NOT NULL,
  last_heartbeat_ack_ms    INTEGER,
  current_status           TEXT NOT NULL,    -- 'active' | 'reconnecting' | 'failed' | 'deregistered'
  push_token_last_sent_ms  INTEGER,
  UNIQUE (relay_url)                          -- one active registration per relay (v1)
);
```

The user inspects "I'm registered with these relays" from Connect; can switch or deregister from there.

## Persistence

The storage's outbound tunnel to the Relay must stay alive long enough for inbound consumer requests to land. Different storage form factors have different cost profiles.

### Always-on storage (NAS, VPS, cloud)

Trivial: the storage process opens an HTTP/3 connection at startup and keeps it open. **Heartbeat every 25 seconds** (under common NAT timeouts). Relay tears down the tunnel after **60 seconds of silence**. **Reconnect with exponential backoff** on drop: `1s → 2s → 4s → 8s → 16s → 30s` cap, capped indefinitely. No special accommodation needed.

### Phone storage (on-device deployment)

Phones can't realistically hold a long-running socket forever — battery, OS doze mode, network changes, OS-level kills of background work. The model:

| Phone state | Tunnel state |
|---|---|
| Connect in foreground | Tunnel open, 25s heartbeat |
| Connect in background, low-power doze | Tunnel may be torn down by OS; Relay holds the registration slot but no live tunnel |
| Wake-up triggered by push notification (Relay → FCM/APNs) | Phone wakes Connect, Connect re-establishes tunnel within seconds |

The Relay holds a **push token** alongside the rendezvous registration. When a consumer attempts to attach and the tunnel is currently torn down, the Relay sends a silent push (FCM data message on Android, APNs background notification on iOS) carrying only "wake up" — no payload, no PII. Connect wakes, re-establishes the tunnel, the consumer's session attaches.

Latency on the first request after a sleep is **a few seconds**; subsequent requests within the same wake window are fast. The session times out and the cycle repeats.

**Android specifically**: a foreground service with a low-priority notification (e.g. "OHD active") can keep the tunnel alive longer; Connect uses this when the user has explicitly opted into "stay always reachable." Otherwise FCM-wake is the path.

**iOS specifically**: stricter limits on background sockets and silent-push frequency. Mitigations:
- Connect maintains the tunnel only when in foreground or freshly woken.
- For non-urgent traffic, accept latency from a push-wake.
- For emergency / break-glass, OS critical-alert push categories deliver more reliably (see [`emergency.md`](emergency.md)).

Connect is responsible for keeping the registration current across device reboots, app updates, and OS push-token rotations. On any of those, Connect calls `Relay.RefreshRegistration` with the new push token.

### Registration revocation

When a user deregisters from a relay (switching relays, decommissioning storage, panic), the storage sends a deregistration RPC; the Relay drops the rendezvous record and any in-flight tunnels. Existing consumer connections through the relay receive a clean close; they retry against the rendezvous URL and now get `404 RENDEZVOUS_NOT_FOUND`. Grant-token holders are expected to surface this to their user as "the user's storage is no longer reachable here."

## Operators

OHD Relay can be deployed and operated by anyone:

- **Project-run Relay**. Our reference SaaS deployment. The default if the user doesn't pick another.
- **Clinic-run Relay**. A hospital or clinic runs their own; in-clinic pairings stay on the clinic's network. External connections never touch the public internet. Operationally simple, fits HIPAA networking expectations.
- **Self-hosted Relay**. Power user with a small VPS runs their own. Often co-deployed with a home OHD Storage (the storage runs at home, the relay runs on a VPS the user controls).
- **Other operators**. A health-aware ISP, a national health service, an insurance group, a community co-op.

A given storage configures one or more Relays; the rendezvous URL in each grant points to the chosen Relay for that grant.

## Auth and accounting

Relay does light auth at the connection level:

- **Per-pairing or per-registration credentials**. Pairing nonces become session-scoped tokens; registration tokens are durable per-grant or per-storage credentials. Relay rejects connections that don't present valid tokens.
- **Optional operator auth**. A clinic-run Relay may require an operator-provided pre-shared key from clients (so only clinic-issued operator devices can attach). Operator policy, not part of the OHD protocol.
- **Bandwidth metering and quota**. Operators meter per-user / per-pairing for billing or abuse prevention.

The relay does not enforce the OHDC grant scope. That's the storage's job, behind the relayed TLS.

## OHDC and Relay

Relay forwards OHDC traffic of any kind — read queries, write submissions, grant management, audit. The auth profile (self-session / grant / device) is end-to-end between the OHDC client and the storage; Relay doesn't distinguish them.

A common pattern that combines several deployment dimensions:

- User's OHD Storage runs on their **home NAS** behind residential NAT.
- User's **OHD Connect mobile app** holds a self-session token. When at home, it talks directly to the NAS over the home network. When traveling, it talks to the same NAS via the user's configured Relay (grant-mediated registration with the user's own self-session token).
- A **Libre CGM service** holds a device token. It talks to the NAS via the same Relay (grant-mediated registration, low-latency from the provider's data center to the Relay endpoint).
- The user's **doctor's OHD Care** holds a grant token. Talks via the same Relay.

All four flows go through the same Relay; the grant tokens / device tokens / self-session tokens determine what each can do once the bytes reach storage.

## Emergency-authority mode

A relay deployment can opt into **emergency-authority mode** — in addition to its packet-forwarding role, it acts as a certified authority that can sign emergency-access requests for break-glass to patient phones. This is what makes the EMS / hospital / clinic emergency-response flow work when paramedics need to access a patient's data without a pre-issued grant.

When operating as an emergency authority, the relay:

- **Holds an authority cert** issued by a trusted root (the OHD project's default root, or a per-country emergency-services CA, etc.). The cert identifies the operator ("EMS Prague Region"); patient phones trust the cert chain to display the operator's certified label in the break-glass dialog.
- **Accepts inbound emergency requests** from authenticated responders (paramedics on the scene, dispatchers at the operator's console). The responder's app authenticates to the relay using the operator's identity system (clinic SSO, hospital ADFS, paramedic-roster auth). The relay is responsible for verifying the responder is currently a member of the operator's roster.
- **Signs emergency-access request payloads** with its authority cert. The signed request travels to the patient's phone (via direct internet, BLE-bystander chain, or any other transport).
- **Routes responder traffic** when the patient's phone has approved the request and issued a grant token. From this point the relay is doing standard packet forwarding — TLS terminates between patient phone and the responder's app endpoint via the relay tunnel.
- **Maintains operator-side audit** of every responder who initiated a break-glass and what they did. This is in addition to the patient-side audit OHD records.

A relay deployment is *not* an emergency authority by default. Authority mode is enabled per-deployment by the operator, with a cert provisioned by the relevant trust root. Most relays stay as plain packet forwarders.

### Authority cert chain

For v1.0:

- The OHD project maintains a default trust root.
- Country / region / institutional emergency-service CAs apply for sub-certs from the OHD root, validated against documented criteria (real organization, real responder roster, regulatory accountability).
- Patient phones ship with the OHD root pre-installed; users can add additional roots (custom country CAs, employer wellness CAs) and remove the default.
- Authority certs include the operator's certified label (the string shown to the user in the dialog), an identifier (`grantee_ulid`-equivalent), and standard PKI validity fields.

This is OHD-managed governance for v1.0. As the project matures, per-country governance and federation with national emergency services will replace the OHD-managed root.

### Bystander-mediated transport

When a patient phone has no internet connectivity (cellular dead zone, airplane mode, depleted radio in unconscious-phone scenarios) but has BLE active, an emergency-authority relay can still reach it via a chain of bystanders:

```
Authority relay  ←→  Internet  ←→  Bystander phone (OHD Connect installed)
                                            ↕ BLE
                                       Patient phone
```

The bystander runs OHD Connect (any OHD Connect installation accepts the proxy role by default). They forward the emergency request and subsequent OHDC traffic transparently. The bystander sees only TLS ciphertext.

This means an OHD-using public is itself a transport asset for emergency response. A passerby with OHD Connect installed is, without any active participation, a usable BLE-to-internet bridge for a nearby unconscious patient's phone. The protocol exploits this without the bystander needing to know it's happening.

## Trust model

| Threat | Mitigation |
|---|---|
| Passive eavesdropping at the relay | TLS end-to-end between storage and client; Relay sees ciphertext only |
| Malicious relay operator | Same as above; Relay can DoS but not read |
| Forged session attach | Pairing nonces are short-lived and single-pairing; registration tokens require valid signatures |
| Token theft via NFC tap proximity | Pairing requires physical proximity; the nonce alone gives only a session, not the OHDC grant or token |
| LAN-side attacker | LAN fast-path uses the same TLS; LAN probe is authenticated against the active session |
| Compromised client endpoint | Storage's OHDC interface still validates the token; compromised client has only what its token allowed |
| Compromised storage endpoint | Pre-existing problem; OHDC tokens and grants live on the storage device regardless of Relay |
| Subpoena / legal compulsion at relay | Relay can be compelled to log sessions and metadata; cannot disclose payloads. Users who care can pick a Relay operator outside hostile jurisdictions, run their own, or pick a deployment topology that doesn't need Relay (custom-provider SaaS, OHD Cloud) |

## Implementation

A Relay service is small. Five tables / state surfaces, nothing else:

| Surface | Fields |
|---|---|
| HTTP/3 server | Any QUIC stack (`quinn` for the reference implementation). |
| **Registration table** (durable) | `(rendezvous_id, user_ulid, current_tunnel_endpoint, push_token, last_heartbeat_at)`. One row per registered user. |
| **Session table** (in-memory + cold-store for telemetry) | `(session_id, rendezvous_id, attached_at, last_active, metering_counters)`. One row per active consumer-attach. |
| **Pairing table** (ephemeral) | `(nonce, expires_at, attached_session_id_or_null)`. For pairing-mediated routing only. |
| Bandwidth meter | Per-user and per-session counters for billing and abuse prevention. |
| LAN-fast-path hints (optional) | mDNS hint forwarding for clients on the same LAN as a registered storage. |

Stateless beyond those tables; horizontally scalable behind a load balancer that pins all sessions for a given `rendezvous_id` to the same instance (consistent-hash by `rendezvous_id`).

The relay holds **no grants, no audit, no tokens, no payload bytes**. A subpoena recovers traffic patterns and the five-field state per user — never content. This minimalism is what makes the relay's privacy property load-bearing.

Distributed as a single Rust binary, fronted by Caddy for TLS. Deployable via Docker Compose alongside (or independently of) an OHD Storage instance.

## Open design items

- **NAT traversal for LAN fast-path**. mDNS works on consumer Wi-Fi but is often blocked on enterprise / clinic networks. Backup discovery via local UDP broadcast or a relay-mediated `STUN`-style hint may be needed.
- **Pairing nonce delivery**. NFC tap is the obvious case but not universal (older devices, no NFC at all on most laptops). QR is the fallback. Bluetooth proximity is also viable. Settle on a small set of supported delivery methods and document the security model for each.
- **Per-grant revocation propagation through active sessions**. When a grant is revoked while a relayed session for that grant is in flight, the storage drops the connection on its end. The relay sees the close and tears down the consumer side. Wire-level signaling for the storage to tell the relay "this rendezvous-attached session is dead" (rather than just letting the TCP/QUIC RST propagate) is not yet pinned.
- **Clinic-run vs project-run trust handoff**. When a patient walks into a new clinic, the clinic's Relay is unfamiliar. The pairing UI should make "this session goes through Clinic X's Relay" explicit and let the user accept or pick an alternative.
- **Multi-relay simultaneous registration**. v1 supports one active relay per user. A future revision may allow forwarding pointers (rendezvous URL on relay A redirects to relay B) for migration grace periods or geographic optimization.
