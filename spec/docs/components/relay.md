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

1. **User issues a grant** via OHD Connect. The grant token includes a "rendezvous URL" pointing at the user's configured Relay.
2. **Storage maintains a long-lived tunnel**. The storage instance opens an HTTP/3 connection out to its configured Relay and registers itself ("incoming connections for grant tokens with rendezvous URL X go to me"). The connection is kept alive with periodic heartbeats. For home servers this is always-on; for phones, this can be either always-on (battery cost) or polled-on-demand (the phone wakes up periodically to check for pending sessions).
3. **Client connects via the rendezvous URL**. The grantee's client (OHD Care, etc.) opens HTTP/3 to the rendezvous URL, presents the grant token (inside its own TLS handshake to the Relay), gets routed to the matching storage tunnel.
4. **TLS handshake end-to-end**. As above; Relay forwards bytes only.
5. **OHDC operations**. As above.
6. **LAN fast-path** if applicable.

The two patterns coexist on the same Relay deployment; storage / client decide which to use based on the grant's metadata.

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

A Relay service is small:

- HTTP/3 server (any QUIC stack).
- Session table: id, two endpoints, created_at, last_active, metering counters.
- Pairing table: nonce, expires_at, allocated session id (or null).
- Registration table: persistent storage-side identifiers for grant-mediated routing.
- Bandwidth meter and rate limiter per user / per pairing / per session.
- mDNS hint forwarding (optional) for the LAN fast-path.

Stateless beyond the session and registration tables; horizontally scalable behind a load balancer that pins both endpoints of a session to the same instance.

Distributed as a single Rust binary, fronted by Caddy for TLS. Deployable via Docker Compose alongside (or independently of) an OHD Storage instance.

## Open design items

- **NAT traversal for LAN fast-path**. mDNS works on consumer Wi-Fi but is often blocked on enterprise / clinic networks. Backup discovery via local UDP broadcast or a relay-mediated `STUN`-style hint may be needed.
- **Pairing nonce delivery**. NFC tap is the obvious case but not universal (older devices, no NFC at all on most laptops). QR is the fallback. Bluetooth proximity is also viable. Settle on a small set of supported delivery methods and document the security model for each.
- **Always-on tunnel battery cost on phones** (grant-mediated mode). Maintaining a persistent tunnel costs power; alternatives are polled-on-demand (phone wakes periodically to check for pending sessions, accepting some latency for incoming queries) or notification-triggered (server-side push wakes the phone when a query arrives).
- **Registration revocation propagation**. When a grant is revoked, the storage needs to disconnect any active relay session for that grant promptly. This is part of the revocation path but the wire details aren't fully specified.
- **Clinic-run vs project-run trust handoff**. When a patient walks into a new clinic, the clinic's Relay is unfamiliar. The pairing UI should make "this session goes through Clinic X's Relay" explicit and let the user accept or pick an alternative.
