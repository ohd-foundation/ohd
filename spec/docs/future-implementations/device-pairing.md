# Future implementation: Device Pairing & Device Tokens

> How sensors, lab providers, pharmacy systems, hospital EHRs, the Health Connect / HealthKit bridge, and other write-only integrations get authorized to push events into a user's OHD storage.

## Status — deferred, post-v1

**This is not a v1 deliverable.** The OHD platform itself (storage, OHDC protocol, OHD Connect, OHD Care, OHD Relay) needs to be running and useful before any third-party integrator will invest in building against it. Detailed device-pairing UX and integrator flows are therefore postponed until that's true.

**What this doc is for**: reserving the design space. The integration models sketched below — companion-app via OS IPC, vendor-backend OAuth, in-app bridges, direct-device — are all paths the v1 architecture must not *block*. The point of this doc is to verify that:

- The `grants.kind='device'` schema in [`../design/storage-format.md`](../design/storage-format.md) supports every integration shape we can foresee.
- The `ohdd_…` token format in [`../design/auth.md`](../design/auth.md) is the right credential primitive.
- Nothing in the OHDC protocol or the auth model has to be re-architected to enable any of the patterns below.

The bit-level pairing flows, IPC contracts, BLE service UUIDs, vendor-registration UX, source-signing canonicalization — all of those are deferred. When real integrator demand surfaces, this doc gets promoted out of `future-implementations/` into `design/` with concrete decisions.

**Most likely primary path** when this lands: companion-app integration via OS-level IPC (Android bound `Service` with custom permission, iOS App Intents) — modeled after how Health Connect itself is integrated. Vendor apps already implement the Health Connect pattern, so adding OHD support is incremental work for them. Direct-device, vendor-backend, and BLE pairing remain real but secondary.

The remainder of this doc is the design-space sketch; treat it as a brainstorm, not a contract.

---

This doc covers **device tokens** — the third auth profile from [`../design/privacy-access.md`](../design/privacy-access.md). It builds on the OAuth and OIDC primitives in [`../design/auth.md`](../design/auth.md); the patterns are the same with one different scope.

## Recap — what a device token is

A device token (`ohdd_…`, see [`auth.md`](../design/auth.md) "Token wire formats") is a specialized grant:

- **Write-only.** It can submit events; it cannot read history, list grants, view audit, or do anything else.
- **No expiry by default.** Sensors run for years; rotating tokens monthly would shred reliability for no security gain.
- **Attributed by `device_id`.** Every event the token writes carries the device's `id`, so users can see "this came from my Libre CGM, that came from my Withings scale."
- **Revocable.** One-click in Connect; sets `grants.revoked_at_ms`; rejected on next use.
- **Modeled in the schema** as a row in `grants` with `kind='device'` (see [`storage-format.md`](../design/storage-format.md)). Queries against `grants WHERE kind='device'` give the user the canonical "connected devices" list.

The damage radius is intentionally bounded: a leaked device token forges events under that device's identity but cannot exfiltrate any history. This is what makes "Libre's CGM service as an OHDC writer" feasible — the auth bar can be low because the consequences of compromise are low.

## Three pairing models

There are three legitimate shapes of "thing that wants to write events on behalf of a user," and each warrants its own pairing flow because the constraints differ.

### Model 1 — User-initiated (QR / pairing code)

The user has a sensor, app, or service in their hand and wants to give it write access to their own OHD storage.

Examples:
- "I just bought a Withings scale; I want it to push weights to my OHD."
- "My partner built a custom Arduino glucose monitor; I want to give it a token."
- "I want my Garmin to push HR samples to my OHD instead of going through Health Connect."

**Trust anchor: physical possession.** The user is in the room with both their phone (running OHD Connect) and the device. The pairing code is short-lived and one-time-use; nobody else can intercept it without proximity.

#### Flow

1. User opens OHD Connect → "Connected devices" → "Pair a new device."
2. Connect calls `Auth.CreatePairingCode(device_kind_hint=...)` over self-session. Storage creates a `pending_pairings` row (system DB; see schema below), returns `(code, qr_url, expires_at_ms)`. Code shape: `BCDF-XYZW` (8 alphanumeric characters, hyphenated) — readable, type-able, low ambiguity (excludes `0/O`, `1/I/L`).
3. Connect displays both the code and a QR encoding `ohd://pair/<code>?storage=<url>`.
4. User pastes / scans into the sensor's setup screen.
5. Sensor opens an HTTPS request: `POST /pairing/redeem` with body `{code, device_metadata: {kind, vendor, model, serial_or_id, app_name, app_version, platform}}`.
6. Storage validates `code` (exists, not expired, not redeemed):
   - Creates a `devices` row from `device_metadata`.
   - Creates a `grants` row with `kind='device'`, `device_id=<above>`, `grantee_label="<vendor> <model>"`.
   - Marks `pending_pairings.redeemed_at_ms`, links to the new grant.
   - Returns `{token: "ohdd_...", device_id, attribution_label, ohdc_endpoint}` to the sensor.
7. Sensor stores the token in its own secure storage; uses it on every subsequent OHDC `PutEvents` call.

The user sees the new device appear in their Connect "Connected devices" list immediately, with the metadata the sensor reported. They can rename the attribution label, revoke, or inspect what it's writing — all from there.

If the user's storage is on-device and not directly reachable, the sensor's HTTPS connection goes through OHD Relay (see [`../components/relay.md`](../components/relay.md)). Same pairing flow; the relay forwards opaquely.

### Model 2 — Provider-initiated (OAuth-style consent)

A vendor (Libre, Dexcom, an EHR, a pharmacy system) wants to push data on behalf of *many* users from their backend service. The vendor doesn't have the user's phone; the user is signing up on the vendor's side.

Examples:
- "I'm setting up my Libre Link account for the first time. Libre's app says: 'do you want to push your CGM data to OHD as well?'"
- "I'm registering at a new lab. They ask if I want results to flow to my OHD."
- "My hospital is integrating with OHD; they ask each patient on discharge whether to push their record."

**Trust anchor: user authentication on OHD's side, plus operator-level vendor registration.** The user logs into OHD through the vendor's flow; the vendor's backend gets a token bound to that specific user.

#### Vendor registration (one-time per integration)

The integrator registers their app with the operator out-of-band — either via the operator's admin UI or via OAuth dynamic client registration (RFC 7591), if the operator allows it:

```
POST /oauth/register
{
  "client_name": "Libre Link Backend",
  "client_uri": "https://librelinkup.com",
  "redirect_uris": ["https://librelinkup.com/ohd/callback"],
  "grant_types": ["authorization_code"],
  "scope": "device.write",
  "device_metadata_template": {
    "kind": "cgm",
    "vendor": "Abbott",
    "model": "FreeStyle Libre 3"
  }
}
→ { "client_id": "...", "client_secret": "...", "client_id_issued_at": ... }
```

Operators can pre-register well-known integrators (so the user sees "Libre" with a verified badge) or accept self-registered ones (less prominent UX).

#### User consent flow

1. User on the vendor's app/site clicks "Connect to OHD."
2. Vendor opens a browser to `https://<storage>/authorize?response_type=code&client_id=<vendor_client>&redirect_uri=...&scope=device.write&state=...&code_challenge=...&code_challenge_method=S256`.
   - Note `scope=device.write` — different from a self-session, this is a request for a device token.
3. OHD Storage:
   - If user has no active session, shows the OIDC login page from [`auth.md`](../design/auth.md). User logs in.
   - Shows a **device-token consent page**: "Libre wants permission to write `glucose`, `glucose_series` events to your OHD as a device named 'FreeStyle Libre 3'. They cannot read your data. They cannot revoke other devices. You can revoke this any time from Connect."
   - User approves (or denies).
4. On approve: OHD Storage creates `devices` row, `grants` row (kind='device'), redirects vendor's browser back with `?code=<one_time>`.
5. Vendor backend exchanges code at `/token`:
   ```
   POST /token
   grant_type=authorization_code&code=<one_time>&client_id=<vendor>&
   client_secret=<vendor_secret>&code_verifier=<pkce>&redirect_uri=...
   → { token_type: "ohd_device", token: "ohdd_...", device_id: 12, ohdc_endpoint: "..." }
   ```
6. Vendor stores `(user_identifier_in_their_system, token, device_id)` in their backend. Uses the token for that user's events from then on.

Notice the response is **not** an OAuth-standard `access_token` + `refresh_token` shape. It's a device token, single-issue, no refresh, no expiry. The OAuth flow is the *issuance vehicle*; the resulting credential follows OHD's device-token contract.

The vendor never sees the user's session token, never gets read scope, never gets grant-management. Just write.

#### Per-event-type scope on device tokens

The vendor declares which event types they intend to write at registration time (`device_metadata_template` and the consent page makes them explicit). The grant row's `grant_write_event_type_rules` is populated with that list. Submissions of other event types are rejected at write time.

This isn't quite the same as a full grant — there's no `default_action`, no read scope, no approval mode. Just a write allowlist. Specifically:

- `grants.kind = 'device'`
- `grants.approval_mode = 'never_required'` (writes commit immediately; no pending queue for sensor data)
- `grant_write_event_type_rules` populated from registration
- All other rule tables empty

If the vendor needs additional event types later (Libre adds glucose-trend events to their schema), they redo the consent flow — user re-approves with the new scope. Pre-existing tokens keep their old scope; the new flow issues a new token with the new scope (vendor migrates).

### Model 3 — In-app pairing (the user's own bridge)

When the OHD Connect mobile app's Health Connect / HealthKit bridge service wants its own attributable token (rather than reusing the user's self-session). Same idea applies to anything else built into the user's apps: a desktop-side bridge for a USB device, the ohd-connect-cli's own auto-poll job, etc.

The user is already authenticated; we're internal to the user's own tooling.

#### Flow

1. The bridge process starts (first launch of Connect mobile, or first time the user enables Health Connect sync).
2. Connect's main process is logged in via self-session. It calls `Auth.IssueDeviceToken(device_metadata={kind: 'phone-bridge', source: 'health-connect', app_name: 'ohd-connect-android', app_version: '...', platform: 'android-14'})` over OHDC under self-session.
3. Storage validates the request (caller has self-session scope `device.issue` — present by default for the user's own apps), creates `devices` and `grants` rows, returns `(ohdd_..., device_id)`.
4. Connect stores the token in EncryptedSharedPreferences / Keystore, separate from the self-session token, with a label that lets the bridge process find it.
5. Bridge process picks up the token; uses it for OHDC `PutEvents` calls when syncing Health Connect → OHD.

Why a separate token rather than reusing self-session: **attribution**. Without it, every Health Connect event would be attributed to "the user themselves" with no way to see "this came from my Galaxy Watch via Health Connect" vs. "this I typed in manually." With a per-bridge device token, attribution is structural.

The bridge token never leaves the device; it's not handed to a third party. Compromise requires device compromise (which already loses everything). The trust bar is low; the convenience is high.

## System-DB tables

### `pending_pairings`

Used only for Model 1 (user-initiated). Lives in the system DB.

```sql
CREATE TABLE pending_pairings (
  id                    INTEGER PRIMARY KEY AUTOINCREMENT,
  code                  TEXT NOT NULL UNIQUE,           -- 'BCDF-XYZW' shape
  user_ulid             BLOB NOT NULL,                   -- which user is pairing
  device_kind_hint      TEXT,                            -- optional: 'cgm', 'scale', 'manual entry'
  created_at_ms         INTEGER NOT NULL,
  expires_at_ms         INTEGER NOT NULL,                -- default now + 10 minutes
  redeemed_at_ms        INTEGER,
  redeemed_grant_id     INTEGER REFERENCES grants(id)    -- set on redemption; cross-table reference into per-user file
);

CREATE INDEX idx_pending_pairings_code
  ON pending_pairings (code) WHERE redeemed_at_ms IS NULL;
```

Pairing codes are single-use and short-TTL (default 10 min, configurable up to 60). After redemption, the row is retained for a configurable retention window (default 30 days) for forensics — useful when a user asks "wait, when did I pair this?".

### `oauth_clients`

Already used by [`auth.md`](../design/auth.md) for client registration; the same table holds vendor registrations for Model 2. Schema lives in `auth.md`; device-pairing reuses it with `scope` including `device.write`.

### `pending_pairings.redeemed_grant_id` cross-reference

Note that `pending_pairings` lives in the system DB but `grants` lives in the per-user file. The `redeemed_grant_id` is therefore *not* a SQL foreign key — it's a logical reference. The storage core enforces the linkage at the API layer.

## Token storage on the device

| Device kind | Where the token lives |
|---|---|
| Mobile app's bridge process | Same platform secure storage as self-session token (Keystore, Keychain). Different keychain item, different access policy. |
| Standalone sensor (Withings, custom Arduino) | Manufacturer's secure-element / encrypted SPI flash. Out of OHD's hands. |
| Vendor backend service (Libre, Dexcom) | Vendor's secrets manager / KMS / encrypted DB. Vendor's responsibility. |
| Lab/pharmacy backend | Same — operator's secrets infrastructure. |
| CLI / scripting tool | `~/.config/ohd/credentials` with `0600` perms; or OS keychain (macOS Keychain, Linux libsecret) if available. |

## Revocation, rotation, audit

### Revocation

User-side: Connect → "Connected devices" → tap a device → Revoke. Sets `grants.revoked_at_ms`. Token rejected on next OHDC call.

A device whose token has been revoked sees `401 Unauthorized` with error code `REVOKED`. Well-behaved integrators: stop pushing and surface "Reconnect to OHD" in their UI. The user re-pairs (Model 1 or 2) to get a new token. Past events written under the revoked token are *not* deleted — they keep their original `device_id` attribution. The user can manually delete them if desired (subject to immutability semantics in [`storage-format.md`](../design/storage-format.md) — they get a soft-delete tombstone).

### Rotation

Device tokens have no expiry, but can be rotated voluntarily:

- For **vendor backends** (Model 2): vendor can call `POST /token/rotate` with the existing token. Storage issues a new token, marks the old as revoked-with-grace (60s overlap window for in-flight requests), updates the same `grants` row's bookkeeping. Useful if the vendor suspects compromise and wants to rotate without making the user re-consent.
- For **user-paired devices** (Model 1): rotation requires a re-pairing. The sensor presents a new pairing code; user approves; new token issued. Different `grants` row, different `device_id`. (We do this rather than in-place rotation so the user has explicit visibility — sensors aren't trustworthy enough to do silent rotation.)
- For **in-app bridges** (Model 3): the app can request a rotation programmatically over self-session; same in-place behavior as vendor rotation.

### Audit

Every event written under a device token produces an audit row tagged `actor_type='grant'`, `grant_id=<the device's grant>`, `query_kind='write'`. The user sees:

- "Glucose 5.4 mmol/L from FreeStyle Libre 3 (Abbott) at 2026-05-07 14:32" — typical event view.
- "Today: 287 events written by FreeStyle Libre 3" — daily summary in audit view.
- "FreeStyle Libre 3 has been silent for 6 hours" — anomaly hint, when expected events stop arriving.

Anomaly detection (events written from unexpected IPs, wildly off-pattern volumes) is an operator-side concern — the audit log captures `caller_ip`, `caller_ua`; downstream tooling flags the rest.

## Optional: source signing for high-trust integrations

Device tokens have a low damage cap by design, but some integrators want to do better — a leaked Libre token forging fake glucose readings would be embarrassing even if it can't exfiltrate. The optional **source-signing** mechanism makes forgery require both the token *and* the integrator's signing key.

How it works:

1. At vendor registration, the integrator generates an Ed25519 key pair. They publish their public key (PEM) to the operator (or via JWKS at a well-known URL on their domain). The operator records `oauth_clients.signing_pubkey`.
2. On every event submission, the vendor signs a canonical hash of the payload (event-type + timestamp + channel values, in deterministic order — the canonicalization is in the conformance corpus) with their private key. The signature is included in the OHDC `PutEvents` call as a per-event field.
3. Storage verifies the signature against `oauth_clients.signing_pubkey` for the client that issued this token. Mismatch → reject.
4. Verified-signature events get `events.metadata.signed_by=<client_name>` set. The user's UI shows "🔒 signed by Libre" on those events.

Signing is **opt-in per integrator**, not enforced. Most integrators won't bother; high-trust ones (clinical labs, regulated medical devices) will. The user's UI can also let them filter or alert on "events from Libre that aren't signed" if they want extra paranoia.

Public-key rotation: integrator publishes a new key, registers it with the operator, signs new events with the new key; the storage validates against any of the integrator's currently-registered keys. Old key gets removed when the integrator is confident no in-flight events still use it.

Bit-level details (canonical-payload encoding, signature container format, operator-side key-pinning policy) are part of the OHDC v1 protocol spec (Task #8); the mechanism is reserved here.

## Multi-tenant integrations — how Libre handles thousands of users

Vendors with many users (CGM providers, lab networks, hospital systems) follow a **per-user-per-vendor** pattern:

- One `oauth_clients` registration: the integrator. ("Libre" registered once with each operator.)
- One `grants` row per (vendor, user): each user separately consents and gets their own `ohdd_…` token.
- Vendor's backend keeps a table: `(libre_user_id, ohd_storage_url, ohd_device_token, ohd_device_id, last_pushed_at)`.

The vendor pushes per-user, sequentially or in parallel, using each user's token. There is no batch endpoint that writes multiple users' events in one call — each user's events go to that user's storage with that user's token, period. This keeps the privacy contract clean (no chance of cross-user contamination at the protocol layer) at the cost of more HTTP traffic on the vendor's side. Most vendors are fine with this; if it ever becomes a real bottleneck, a future revision could explore batched-with-sharded-tokens (still one token per user, but framed in one HTTP call). Not v1.

## Health Connect / HealthKit bridge — the canonical Model-3 case

The OHD Connect mobile app's bridge component is one of the largest device-token consumers in the spec. It runs as a long-lived background service, polls Health Connect / HealthKit on a schedule, translates records to OHD events, and pushes them via `PutEvents` over its own device token.

What makes the bridge interesting:

- **Per-source attribution.** Health Connect aggregates from many apps (Samsung Health, Garmin Connect, Libre Link, etc.). The bridge token's `device_id` identifies the bridge itself; `events.metadata.source` carries the upstream `dataOrigin.packageName` so users can see "this came via Health Connect from Samsung Health." Users who want a single bridge but per-source attribution can rely on metadata; users who want per-source `device_id`s can configure per-source bridges (Phase 2+).
- **Idempotency.** Health Connect change-token replay can produce duplicate events. The bridge derives `source_id` deterministically from `(dataOrigin.packageName, recordId)` so OHD's `idx_events_dedup` UNIQUE constraint absorbs duplicates.
- **Backfill.** First-launch backfill (last 90 days) is a one-shot bulk write; subsequent incremental syncs use change tokens. The bridge's behavior is described operationally in [`../research/health-connect.md`](../research/health-connect.md); the auth shape is just Model 3.

## Cross-references

- Three-auth-profile overview: [`privacy-access.md`](../design/privacy-access.md)
- Self-session OAuth + OIDC mechanics, token formats, system-DB tables: [`auth.md`](../design/auth.md)
- Per-user `grants` and `devices` schema: [`storage-format.md`](../design/storage-format.md)
- OHDC operations including `Auth.IssueDeviceToken`, `/pairing/redeem`, pairing/consent UX: [`../components/connect.md`](../components/connect.md)
- Relay (which carries pairing traffic when storage is on-device): [`../components/relay.md`](../components/relay.md)
- Health Connect / HealthKit bridge operational details: [`../research/health-connect.md`](../research/health-connect.md)

## Open items (deferred)

- **Per-source `device_id`s** for the Health Connect bridge (one bridge token per upstream source app rather than one bridge token total). Phase 2+.
- **Batched multi-user writes** for very-high-fanout integrators. Only if measured bottleneck.
- **Source-signing canonicalization** — the deterministic encoding of a payload for signature input. Lives in OHDC v1 protocol spec (Task #8) when that gets written.
- **Vendor-side OAuth flows for users who don't yet have an OHD account.** Today, if Libre asks "do you want to push to OHD?" and the user doesn't have an account, the user has to sign up first elsewhere, then come back. A future revision could pre-create accounts on OHD Cloud directly from a vendor's flow with an account-creation scope. Not v1.
