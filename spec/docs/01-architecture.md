# 01 — Architecture

OHD is a **four-component** system. **OHD Storage** is the core data layer, exposing a single external protocol (**OHDC**). Three other components surround it:

- **OHD Connect** — the personal app that uses OHDC under self-session auth.
- **OHD Care** — the reference clinical app that uses OHDC under grant-token auth.
- **OHD Relay** — the bridging service that lets remote OHDC consumers reach storage instances behind NAT (phones, home servers without public IP).

```
┌─────────────────────────────────────────────────────────────────────────┐
│                            OHD STORAGE                                  │
│                                                                         │
│   on-disk format  │  permissions  │  audit  │  grants  │  sync          │
│                                                                         │
│   ┌─────────────────────────────────────────────────────────────┐       │
│   │   OHDC protocol — three auth profiles                       │       │
│   │     • self-session  (full scope on own data)                │       │
│   │     • grant token   (scoped read / write, optional approval)│       │
│   │     • device token  (write-only, attributed by device)      │       │
│   └────▲──────────────────────▲──────────────────────▲─────────-┘       │
│        │                      │                      │                  │
└────────┼──────────────────────┼──────────────────────┼──────────────────┘
         │                      │                      │
         │                      │ ┌─── OHD Relay ───┐  │
         │                      │ │  bridges remote │  │
         │                      │ │  consumers ↔    │  │
         │                      │ │  unreachable    │  │
         │                      │ │  storage        │  │
         │                      │ └─────────────────┘  │
         │                      │                      │
   ┌─────┴─────────────┐ ┌──────┴───────────────┐ ┌────┴──────────────────┐
   │ OHD Connect       │ │ OHD Care             │ │ Third-party           │
   │ (personal app)    │ │ (clinical EHR-shape) │ │ integrations          │
   │                   │ │                      │ │                       │
   │ • Android / iOS   │ │ • Web app            │ │ • Libre / Dexcom CGM  │
   │   / web           │ │ • Multi-patient      │ │ • Lab providers       │
   │ • OHDC CLI        │ │ • Care MCP           │ │ • Pharmacy systems    │
   │ • Connect MCP     │ │ • Care CLI           │ │ • Hospital EHRs       │
   │                   │ │ • Write-with-        │ │                       │
   │ Health Connect /  │ │   approval flow      │ │ Each holds a per-user │
   │ HealthKit bridge, │ │                      │ │ device token; pushes  │
   │ manual logging,   │ │ Holds grant tokens   │ │ events on schedule.   │
   │ personal dash     │ │ per patient          │ │                       │
   └───────────────────┘ └──────────────────────┘ └───────────────────────┘
```

## OHD Storage — the core product

**Responsibilities.** Storing health events over a person's lifetime, validating writes against the channel registry, enforcing access control, recording audit, syncing between deployments, holding the channel registry, encrypting data at rest. The "database product" — not a generic database, a single-purpose typed store for health events.

**Form factor.** A library + service. Single on-disk format and single interface contract everywhere; only the deployment topology differs. See [`design/storage-format.md`](design/storage-format.md) for the format and [`components/storage.md`](components/storage.md) for the component spec.

**Where it runs.** Four deployment modes, all using the same code. The user picks at app setup; see [`deployment-modes.md`](deployment-modes.md) for the user-facing tradeoffs.

| Mode | Operator | Reachable directly? |
|---|---|---|
| On the user's phone (on-device) | The user | No — needs OHD Relay for external access |
| OHD Cloud | The OHD project | Yes |
| Custom provider (clinic / insurer / employer / etc.) | The third party | Yes |
| Self-hosted (VPS / NAS / home server) | The user | Maybe — needs OHD Relay if behind NAT |

**External surface.** Storage exposes one protocol — OHDC. There is no "raw API." OHDC is the only contract every consumer (the OHD Connect app, OHD Care, sensor integrations, MCP servers) speaks against.

## OHDC — the protocol

A single typed protocol for everything: read, write, aggregate, export, manage grants, view audit, run pending-event approvals. What an authenticated session can actually invoke is determined by its **token's auth profile**, not by the protocol layer. Three auth profiles flow through the same OHDC API:

### Self-session auth

The user authenticated as themselves via OIDC. Full scope on their own data — every operation available, full filter capability, grant management, audit inspection.

Used by: OHD Connect personal app, OHDC CLI, Connect MCP.

### Grant-token auth

A user-issued grant for a third party. Scope is bounded by structured rules in the grant: read scope (event types / channels / sensitivity classes / time windows), write scope (which event types the grantee can submit), approval policy (`always` / `auto_for_event_types` / `never_required`), expiry, rate limits.

Used by: OHD Care, researcher portals, family / delegate access.

The auth bar is high (the user has to deliberately issue a grant) because the damage cap is high — leaked grant tokens disclose what the grant allowed. Revocation is synchronous, not sync-deferred.

### Device-token auth

A specialized grant: write-only, no expiry, attributed by `device_id`. Issued during one-time pairing (QR / OAuth / NFC).

Used by: sensor / CGM integrations (Libre, Dexcom), lab providers, pharmacy systems, hospital EHRs pushing data, the OHD Connect mobile app's Health Connect / HealthKit bridge worker.

The auth bar is low because the damage cap is low — leaked device tokens can forge events under the device's identity but cannot exfiltrate. This is what makes "Libre's backend as a writer" feasible.

### Why one protocol, not multiple

Earlier drafts split read/write/integration into separate protocols. The unified model is cleaner because mixed-scope grants (a doctor with read+write+approval) become one grant, not synthetic combinations. The OHDC API stays uniform; auth determines capability. See [`components/connect.md`](components/connect.md) for the full protocol spec.

## OHD Connect — the personal app

The canonical personal-side reference. Android, iOS, and web. Authenticated to OHDC under a self-session token; runs against the user's storage (in-process if local, HTTP/3 if remote).

What it does:

- **Logging** — Health Connect / HealthKit bridge, manual entry (barcode food via OpenFoodFacts producing parent meal + child food_items, medications, custom measurements), voice / free-text input, symptoms.
- **Personal dashboard** — recent activity, charts, timelines, saved views.
- **Grant management** — create / revoke / inspect grants, see what each grantee has queried, see what was silently filtered.
- **Pending review** — when a grant submits a write under the approval queue, the user reviews via OHD Connect.
- **Export / portability** — full portable export, doctor-PDF, migration to a different deployment mode.

Plus an MCP server (Connect MCP) and a CLI (`ohd-connect`) for LLM- and terminal-driven workflows respectively.

See [`components/connect.md`](components/connect.md) for details.

## OHD Care — the reference clinical app

A real, usable, lightweight EHR-shaped consumer. Open-source. Designed for OHD-data-centric clinical workflows in:

- A specialist's office whose main EHR doesn't speak OHDC yet — Care for the visit, the EHR for billing/scheduling.
- A small clinic, direct-pay practice, mobile / home-care service, ambulance crew, clinical trial site.
- Any operator who wants to demonstrate "you own the data; we just access what you grant us."

Care is **not** competing with Epic / Cerner. It's focused on the OHDC clinical workflow, deployed in an afternoon, alongside (or instead of) other EHRs.

Distinctive features:

- **Multi-patient roster** — every patient who has granted the operator access. Active-patient context drives every operation.
- **Visit prep** — opens a patient and pre-fetches the data slices likely to matter for their current concern (temp series for "feeling sick", recent meds, related symptoms).
- **Write-with-approval** — submissions (lab results, observations, clinical notes, prescriptions) go to the patient's pending queue. Trust-tiered policy lets long-term primary doctors auto-commit routine writes while still queueing high-stakes ones.
- **Care MCP** — multi-patient LLM workflow. `switch_patient(label)` sets the active grant; subsequent tool calls scope to that patient.

See [`components/care.md`](components/care.md) for the full spec.

## OHD Relay — the bridge

Forwards opaque packets between OHDC consumers and storage instances that can't accept inbound connections — phones (NAT, sleep, mobile networks), home servers behind residential NAT or CGNAT, anything without a public IP.

Two routing patterns:

- **Pairing-mediated**: in-person handshake (NFC tap, QR scan) at the desk. Short-lived sessions. Trust anchor is physical proximity.
- **Grant-mediated**: storage maintains a long-lived tunnel registered against a rendezvous URL embedded in the grant. Remote consumers connect to the rendezvous URL and Relay forwards. Trust anchor is the grant token.

Relay sees ciphertext only. TLS terminates at storage and at consumer; Relay forwards bytes. Operators include: us (project-run), clinics, self-hosted by the user, third-party (national health services, ISPs).

See [`components/relay.md`](components/relay.md) for the full spec.

## How the components interact

### Write path (any OHDC consumer)

```
User does something (takes pill, scans barcode, sensor produces reading...)
  → OHDC consumer (Connect app, Care, CGM service, CLI, MCP, ...)
  → translates to typed Event(s) — registry validation
  → put_events via OHDC (in-process if local, HTTP/3 if remote, via Relay if storage unreachable)
  → Storage validates token + auth profile
  → If grant token with approval-required policy: event → pending_events; user notified
  → Otherwise: event → events with new ULID
  → Audit row appended
  → Wire ULIDs returned to consumer
```

### Read path (self-session — own data)

```
User opens personal dashboard
  → OHD Connect issues query via OHDC with self-session token
  → Storage skips permission intersection (full scope), runs query, audits
  → Typed events returned
```

### Read path (grant token — third party)

```
Doctor opens patient in OHD Care
  → Care issues query via OHDC with grant token
  → Storage resolves grant, intersects with rules (event types / channels / sensitivity / time)
  → Filtered events returned; rows_filtered count audited (visible to user)
  → Audit row records the grant_id, query, and result
```

### Live access for on-device storage

```
Patient and doctor at the desk
  → NFC tap pairs phone with doctor's OHD Care device
  → Phone opens HTTP/3 session out to its configured Relay
  → Doctor's OHD Care opens HTTP/3 session in to the same Relay
  → TLS handshake end-to-end through the relay
  → OHDC operations flow with the grant token
  → LAN fast-path probes; on success, session migrates off Relay onto direct LAN
  → When the phone disconnects, session ends
```

## Why four components

1. **Single protocol surface.** OHDC is the one external contract. Easy to learn, easy to integrate, no synthetic combinations of multiple protocols for mixed-scope grants.
2. **Auth-asymmetric scopes.** Self-session, grant, and device tokens have different damage caps. The protocol stays uniform; auth scope determines capability. This lets a cheap-to-issue device token coexist with a high-bar grant token without protocol-level distinctions.
3. **Trust separation by component.** OHD Connect runs under user's full control. OHD Care holds grants the user issued and is operator-deployed. OHD Relay sees ciphertext only. Each component's compromise has a bounded blast radius.
4. **Real apps, not just protocols.** OHD Connect (personal) and OHD Care (clinical) are reference implementations users actually run. Care is positioned as a real, lightweight, usable EHR-shaped consumer — not a demo, not a competitor to enterprise EHRs.

## Protocol versioning

OHDC is versioned. Storage advertises supported versions; consumers pick the highest mutually supported. Breaking changes require a major bump and a documented migration. Additive changes don't.

The portability promise — "any OHD instance can import any other instance's export" — applies across operators, across versions (within compatibility windows), and across deployments. The export format is the durable contract; components implement it.
