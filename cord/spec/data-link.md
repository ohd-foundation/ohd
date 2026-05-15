# Design: the CORD data link — Shares & the relay data plane

> How a remote consumer — CORD, or a clinician's device — reaches a user's
> health storage. This is the piece that is currently **unbuilt** and is the
> reason "finish CORD" is non-trivial. Companion to [`../SPEC.md`](../SPEC.md)
> and the relay wire spec
> [`../../relay/spec/relay-protocol.md`](../../relay/spec/relay-protocol.md).

## The problem

A beta user's OHD storage lives **on their phone** — an encrypted SQLite +
chunk files inside OHD Connect (`deployment-modes.md` "On-device"). CORD runs
server-side at `cord.ohd.dev`. For CORD to answer "how was my glucose last
week?" it must read that phone-resident store, across NAT, without CORD ever
holding the user's keys and without the relay seeing plaintext.

The building blocks already exist — OHD Relay (NAT tunnel, deployed), the
grant model (OHD Storage), `ohd-mcp-core` (the tool catalog). What is missing
is the **wiring**: a user-facing way to mint and hand over a scoped
remote-access credential, and a phone-side endpoint that answers scoped
requests over the tunnel.

## Shares — the user-facing model

A **Share** is the single concept the user manages: *"this party may see this
slice of my data."* It unifies two things that are separate in the code today
— OHD grants and the emergency break-glass profile — into one mental model
and one screen.

A Share = **a grant** (the OHDC scope object: read/write rules, sensitivity
classes, time window, expiry) **plus an optional remote-access binding** (a
relay rendezvous + connection artifact) when the grantee needs asynchronous
access rather than in-person.

### Connect UI: a first-class Shares tab

Sharing is currently buried inside a "Profile & Access" screen. It is one of
the most important things the app does and gets a **top-level tab**.

The Shares tab is a list. Each row:

- Label (grantee name — "Dr. Smith", "CORD", "Emergency").
- Type chip (doctor / family / researcher / agent / emergency).
- Status line — scope summary, expiry, last access.
- A **quick enable/disable toggle** — instantly suspends/resumes the share
  without deleting it (storage flips the grant's `suspended_at_ms`; a
  suspended grant resolves to "deny all" without losing its rules).

Tapping a row opens **share detail**: the full scope (event types, channels,
sensitivity classes, time window), the per-share audit, remote-access status,
and actions (edit scope, re-issue link, revoke).

**Emergency is a first-class share.** The emergency break-glass profile is
modeled as a pre-configured share, `kind = emergency`, pinned to the top of
the list, non-deletable. It carries everything a normal share does *plus* the
break-glass extras — approval timeout, default-on-timeout, trusted-authority
roots, history window — surfaced in its detail screen. A future addition,
noted here so the screen is designed for it: an info/action affordance
explaining how to enable the BLE proximity beacon ("the prompt we discussed
for an in-office responder").

This makes the model uniform: emergency is not a separate subsystem, it is
*the share with break-glass semantics*. New share kinds (a research study, an
employer wellness program) slot into the same list.

## Activating remote access on a share

By default a grant is in-person only (the grantee must be on the same network
or hold a direct URL). To make a share reachable asynchronously the user
**activates remote access** inside share detail:

1. **Pick a relay.** Default: OHD Relay (`relay.ohd.dev`). The user may enter a
   **custom relay** — a clinic running its own. (When the BLE proximity beacon
   is enabled, the beacon can carry the relay config so an in-office responder
   is configured automatically — future; see "BLE-assisted config".)
2. **Connect registers a per-share rendezvous.** OHD Connect — via the storage
   core — calls the relay's `POST /relay/v1/register`, obtaining a
   `rendezvous_id` and `long_lived_credential` *scoped to this share*. Each
   share gets its **own** rendezvous (the "per-grant sub-rendezvous"
   open item in `relay-protocol.md` — adopted here): revoking one share's
   remote access never disturbs another.
3. **Connect opens/maintains the tunnel.** The phone keeps a relay tunnel for
   any share with remote access enabled, re-established on wake via push
   (`WAKE_REQUEST`, `relay-protocol.md` §frame `0x09`).
4. **A share link is produced** — the artifact below.

Disabling remote access deregisters the rendezvous; revoking the share also
revokes the underlying grant.

## The share link artifact

The artifact the user hands to a grantee. One logical credential, four
carriers (the user picks per situation): a custom-scheme deep link, an
`https://` mirror, a QR code, an NFC payload.

**Canonical form** (extends the `ohd://grant/...` artifact in
`relay-protocol.md` §"Pin format" with an explicit share binding):

```
ohd://share/<rendezvous_id>?token=<ohdg_…>&pin=<sha256_spki_b64url>&relay=<relay-host>[&case=<ulid>]
```

| Param | Meaning |
|---|---|
| `<rendezvous_id>` | the per-share rendezvous on the relay (opaque; does **not** embed the user ULID — see note). |
| `token` | the `ohdg_…` grant token. This **is** the bearer credential. |
| `pin` | SHA-256 of the storage's identity-key SPKI; the cert-pinning trust anchor. |
| `relay` | the relay host. OHD's or a custom one. |
| `case` | optional, when the share is case-bound. |

**Carriers:**

- **Custom scheme** — `ohdr://share/...` (registered by OHD Connect, OHD Care,
  and resolvable by CORD's link handler). Same query string as the canonical
  form. This is the "tap to open in the OHD app" path.
- **`https://` mirror** — `https://<relay-host>/share/<rendezvous_id>#token=…&pin=…`
  — for devices/contexts with no custom-scheme handler (a clinician opening
  the link in a browser, which then offers "open in CORD"). Credentials go in
  the URL **fragment** (`#…`), never the query, so they are not sent to the
  relay's web server or logged.
- **QR** — encodes the canonical `ohd://share/...` URL. Connect renders it in
  share detail for in-person handover.
- **NFC** — an NDEF record carrying the same URL, for tap-to-share.

> Privacy note on the URL shape. The intent expressed in design discussion was
> `relay.ohd.dev/{user_ulid}?share={ulid}`. We deliberately use an **opaque
> `rendezvous_id`** instead of the raw `user_ulid`: a share link is sometimes
> shown on screens, photographed, or pasted, and the user ULID is a stable
> cross-share correlator. The opaque per-share id carries no such linkage and
> is independently revocable. The relay maps `rendezvous_id → (user, share)`
> internally. Functionally identical; strictly more private.

## CORD connecting a data source

When the user gives CORD a share link (`POST /v1/sources/connect`, or by
opening an `ohdr://` link that CORD's handler routes to that endpoint):

1. **Parse** the artifact → `(rendezvous_id, token, pin, relay)`.
2. **Open the relay tunnel.** CORD connects to
   `https://<relay>/r/<rendezvous_id>`, exchanges `HELLO`, presents `token` in
   the HELLO payload. The relay assigns a `SESSION_ID` and sends `OPEN` to the
   phone.
3. **Inner TLS + pin.** Inside the tunnel CORD and the phone do a TLS 1.3
   handshake; CORD verifies the storage cert's SPKI fingerprint against `pin`.
   Mismatch → abort, surface "this storage isn't who the share said it would
   be" (`relay-protocol.md` §"TLS-through-tunnel").
4. **MCP handshake.** Over the inner-TLS session CORD speaks MCP — `initialize`
   then `tools/list`. The phone answers with the share-scoped catalog.
5. **Store the credential.** CORD persists `(rendezvous_id, token, pin, relay,
   label, scope summary)` **encrypted at rest** (`OHD_CORD_DATA_KEY`). The
   token is long-lived; CORD reconnects on demand for each chat.

From here every chat turn that needs data runs MCP `tools/call` over a fresh
(or pooled) tunnel session to that source.

## The phone-side share responder

The genuinely new capability on the phone. Today OHD Connect for Android
embeds `ohd-storage-core` + `ohd-mcp-core` via uniffi and runs CORD's tool
loop **locally, as the owner** — full, unscoped access. It has no way to
*serve* a remote consumer and no way to enforce a grant's scope on tool
output. Both are needed.

### Responder

A background component in OHD Connect — the **share responder** — that, for
every share with remote access enabled:

1. Maintains the relay tunnel (register, `OpenTunnel`, heartbeat, push-wake
   re-establish).
2. Terminates the **inner TLS** server side (a `rustls` server using the
   storage's self-signed identity cert — the consumer pins it).
3. Speaks **MCP** over that session: `initialize`, `tools/list`, `tools/call`.

The relay client + tunnel framing already exist in
`ohd-storage-server/src/relay_client.rs` and the relay's `frame.rs`. Phase 4
extracts the client side into a crate reusable from the Android uniffi
binding (Android embeds the core, not the server binary).

### Scope enforcement

The responder must **not** call `ohd-mcp-core` as the owner. The share's
grant scope is applied to every tool call:

- `ohd-mcp-core::dispatch` is parameterized with an optional **`ShareScope`**
  (derived from the grant's `read_rules` / `channel_rules` /
  `sensitivity_rules` / time window). When present, dispatch intersects every
  query's `EventFilter` with the scope and redacts out-of-scope channels from
  results.
- `tools/list` returns only the tools the scope allows — write tools omitted
  unless the grant has write rules.
- Out-of-scope requests return an MCP error the agent is told to treat as
  "not permitted", never "no data".

This makes the phone the **enforcement boundary**: a compromised or buggy
CORD cannot exceed what the user granted. CORD's own scope checks are
defense-in-depth only.

Every `tools/call` appends one audit-log row under the share's grant, exactly
as an OHDC RPC would — the user sees CORD's activity in the same audit view as
any clinician's.

## Why MCP over the tunnel (not OHDC)

The relay tunnel is transport-agnostic — it forwards opaque `DATA` frames; the
inner TLS session can carry anything. Two candidates for what rides inside:
**OHDC** (the storage RPC protocol) or **MCP** (the tool protocol). CORD uses
**MCP**, because:

- The phone has `ohd-mcp-core` (the tool catalog) but **not**
  `ohd-storage-server` (the OHDC server + grant-resolution layer). Serving
  OHDC from the phone would mean embedding the whole server stack into the
  Android binding. Serving MCP only needs the responder above.
- CORD's agent is a tool-use loop; it wants `tools/list` / `tools/call`, not
  raw `QueryEvents`. MCP is the natural fit and reuses the catalog Android
  CORD already drives.
- A clinician-facing OHDC consumer (OHD Care) can still get a tunnel to the
  same phone — that path stays OHDC and is Care's concern, out of scope here.

Cost of the choice: grant-scope enforcement, which OHDC gets "for free" in the
server layer, must be implemented in the responder (the `ShareScope` above).
That is a bounded, well-contained change to `ohd-mcp-core`.

## BLE-assisted config (future)

When the emergency share's BLE proximity beacon is enabled, the beacon can
advertise the relay configuration (relay host + a short-lived pairing nonce).
An in-office responder's device, on detecting the beacon, is pre-filled with
the relay to use — collapsing the "type the relay URL" step. This rides the
pairing-mediated pattern in `relay-protocol.md` §"Pairing-mediated"; the wire
shape is unchanged. Not in the initial build; the Shares/emergency detail
screen is designed with room for the toggle.

## Build phases (data link only)

Maps onto the roadmap in [`../SPEC.md`](../SPEC.md):

- **Connect — Shares tab (roadmap Phase 3).** UI rework: promote sharing to a
  top-level tab, per-share toggle, detail screen, emergency as a first-class
  share. Share-link generation (`ohd://share/...`, QR, NFC). Grant
  `suspended_at_ms` for the toggle; per-share rendezvous registration.
- **Relay data plane (roadmap Phase 4).** Extract the relay client crate; the
  phone-side share responder (tunnel + inner-TLS server + MCP); `ShareScope`
  in `ohd-mcp-core`; CORD's relay MCP client; cert pinning end to end.
- **Hardening (roadmap Phase 5).** Custom relay, push-wake, per-share metering.

## Cross-references

- CORD service spec: [`../SPEC.md`](../SPEC.md)
- Relay wire protocol (tunnel, framing, cert pinning, registration): [`../../relay/spec/relay-protocol.md`](../../relay/spec/relay-protocol.md)
- Grants, scope dimensions, emergency break-glass: [`../../storage/spec/privacy-access.md`](../../storage/spec/privacy-access.md)
- Grant artifact / `ohd://` format: [`../../relay/spec/relay-protocol.md`](../../relay/spec/relay-protocol.md)
- Tool catalog + dispatch: `storage/crates/ohd-mcp-core`
- Connect app (issuing side): [`../../connect/SPEC.md`](../../connect/SPEC.md)
