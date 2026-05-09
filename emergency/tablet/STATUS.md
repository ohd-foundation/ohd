# `tablet/` — Status & Implementation Handoff

> Snapshot of where the OHD Emergency paramedic tablet app is. Read this
> first if you're picking up implementation.

## OHDC wire/API version renamed to v0 (2026-05-09)

Tablet comments and proto keep-rule examples now use the pre-stable
`ohdc.v0` API namespace.

## Real OHDC + real BLE wired (2026-05-09)

The four big stubs — `initiateBreakGlass`, `loadPatientView`,
`submitIntervention`, `loadTimeline`, `handoffCase` — now ride a real
hand-rolled OHDC Kotlin client (OkHttp + Connect-Protocol JSON) and a
real `BluetoothLeScanner`-backed beacon discovery path. The mock
fallbacks remain in place for the dev/demo environment but are no
longer the primary path.

See [BUILD.md](BUILD.md) "OHDC client" + "Real BLE scan" + "Real
/v1/emergency/initiate" + "Persistent offline-write queue" for what
landed.

## Phase

**v0 wired against real OHDC + relay.** All routes, all screens, full
break-glass + intervention + handoff happy-path. Mock fallbacks remain
for the demo environment (no relay running) but the real wire is the
primary path.

## What's UI'd (and works against real wire / mock fallback)

| Surface | Status | Where |
|---|---|---|
| Operator sign-in (real OIDC, mirroring connect/cli + connect/web pattern) | OIDC wired via AppAuth-Android; tokens in EncryptedSharedPreferences; legacy stub kept for dev | `LoginScreen.kt`, `OidcManager.kt`, `OperatorSession.kt` |
| Patient discovery via BLE | ✅ Real `BluetoothLeScanner` against placeholder OHD UUID + mock fallback when no permission / no BLE | `DiscoveryScreen.kt`, `BleScanner.kt` (`RealBleScanner` / `MockBleScanner`) |
| Manual-entry fallback | UI complete | `DiscoveryScreen.kt` (manual-entry dialog) |
| Active-case banner on Discovery | UI complete; resumes via NavHost | `DiscoveryScreen.kt` |
| Break-glass confirm screen | UI complete | `BreakGlassScreen.kt` |
| Break-glass countdown | UI complete; 1Hz ticker; cancellable | `BreakGlassScreen.kt` |
| Auto-grant resolution + chip | UI complete; 5s mock | `BreakGlassScreen.kt`, `CaseVault.kt` |
| Patient view header (case ULID, elapsed, auto-grant chip) | UI complete | `PatientScreen.kt` |
| Critical info card (red border) | UI complete | `CriticalCard.kt` |
| Active medications | UI complete | `PatientScreen.kt` |
| Recent vitals row + sparklines | UI complete; Canvas-drawn | `PatientScreen.kt` |
| Active diagnoses | UI complete | `PatientScreen.kt` |
| Recent observations | UI complete | `PatientScreen.kt` |
| "Hide non-emergency data" toggle | UI complete; switch wired (filter pass-through TODO) | `PatientScreen.kt` |
| Intervention quick-entry cards | UI complete (HR / BP / SpO2 / Temp / Drug / Observation / Note) | `InterventionScreen.kt`, `QuickEntry.kt` |
| Vitals number pad | UI complete; chunky 3×4 grid | `VitalsPad.kt` |
| Drug administration form | UI complete | `InterventionScreen.kt` |
| Submission "Logged: …" toast | UI complete | `InterventionScreen.kt` |
| Case timeline | UI complete; filter chips | `TimelineScreen.kt` |
| Queued-not-flushed badges | UI complete | `TimelineScreen.kt`, `CaseVault.kt` |
| Top bar (operator / sync / panic-logout) | UI complete | `TopBar.kt` |
| Sync indicator chip | UI complete; Synced / Queued N / Syncing / Offline | `SyncIndicator.kt` |
| Handoff facility picker | UI complete (3 mock destinations + manual entry) | `HandoffScreen.kt` |
| Handoff confirm + success screen | UI complete; transitions back to discovery | `HandoffScreen.kt` |
| Panic-logout from any screen | UI complete; clears OperatorSession + CaseVault | `MainActivity.kt`, `EmergencyRepository.panicLogout()` |
| NavHost routes | Complete: `/login → /discovery → /break-glass/{beaconId} → /patient,/intervention,/timeline,/handoff/{caseUlid}` | `Routes.kt`, `MainActivity.kt` |
| Dark Material3 theme | Complete; dynamic colour intentionally disabled | `Theme.kt`, `Color.kt` |
| Tablet-friendly type scale | Complete; +25% display, +15% body over Material3 default | `Type.kt` |
| Permissions in manifest | Complete (BLUETOOTH_SCAN neverForLocation, BLUETOOTH_CONNECT, legacy BLE pre-12, INTERNET, FOREGROUND_SERVICE, WAKE_LOCK, POST_NOTIFICATIONS, USE_BIOMETRIC) | `AndroidManifest.xml` |
| Backup-exclusion rules | Complete (`case_vault.db*`, `ohd_emergency_secure.xml`) | `data_extraction_rules.xml` |
| ProGuard keep rules | Complete (uniffi + JNA) | `proguard-rules.pro` |

## What's wired (and where the mock fallback still kicks in)

| Surface | Wired | Demo-fallback |
|---|---|---|
| BLE scan | ✅ `RealBleScanner` against `BluetoothLeScanner`, parsing service-data under the placeholder OHD UUID. Permission flow via `rememberLauncherForActivityResult` from `DiscoveryScreen`. | `MockBleScanner` (3 mock patients in ~3s) when `BLUETOOTH_SCAN`/`ACCESS_FINE_LOCATION` runtime permission is missing or no BLE adapter is available. The canonical OHD beacon service UUID is TBD (`spec/emergency-trust.md` "Open items") — `PLACEHOLDER_OHD_SERVICE_UUID` constant in `data/BleScanner.kt` is one constant change away from production. |
| Operator OIDC | ✅ AppAuth-Android (`net.openid:appauth:0.11.1`) Code + PKCE Custom-Tab flow; bearer + refresh + AppAuth state JSON persist via `EncryptedSharedPreferences`. | Stub sign-in via `OperatorSession.stubSignIn` for offline dev. |
| `/v1/emergency/initiate` (relay-private) | ✅ `OhdcClient.emergencyInitiate(...)` POSTs the real wire shape (mirrors `relay/src/server.rs::handle_emergency_initiate`). After initiate, polls `/v1/emergency/status/{request_id}` once/sec until terminal state. | When the relay returns 404 / network error (relay's status endpoint isn't yet wired — see relay STATUS.md), `EmergencyRepository.initiateBreakGlass` falls back to a 5s synthetic `AutoGranted` so the demo continues. |
| OHDC `QueryEvents` (patient view) | ✅ `OhdcClient.queryEvents(...)` Connect-Protocol streaming with `event_types_in` set to the emergency profile (`std.allergy`, `std.blood_type`, `std.advance_directive`, `std.medication`, `std.diagnosis`, `std.vital`, `std.observation`). Maps the streamed events into `PatientView` panels. | `MockPatientData.exampleView` when QueryEvents fails or returns empty. |
| OHDC `PutEvents` (intervention writes) | ✅ `OhdcClient.putEvents(...)` Connect-Protocol unary. Channel-path mapping per `EmergencyRepository.payloadToEventInput`: vitals → `std.vital`, BP → `vital.bp_sys` + `vital.bp_dia`, drugs → `std.medication.administered`, observations → `std.observation`, notes → `std.note`. On `Committed` outcome the queued write is flushed. | On HTTP failure / `Pending` / per-row `Error`, the write stays queued and the SyncIndicator shows `Queued (N)`. |
| OHDC `QueryEvents` (timeline) | ✅ Same streaming RPC; merged with `CaseVault.queuedWrites` overlay sorted descending by timestamp. | `MockPatientData.exampleTimeline` baseline when QueryEvents fails. |
| `/v1/emergency/handoff` (relay-private) | ✅ `OhdcClient.emergencyHandoff(...)` POSTs the JSON wire (provisional shape; relay endpoint TBD per relay STATUS.md). | Mock OK with synthetic successor case ULID + read-only grant when the endpoint is unavailable. |
| Persistent offline-write queue | ✅ `QueuedWriteStore` backs `CaseVault.queuedWrites` with SQLite (`case_vault.db`). Survives reboot; loaded into memory on `attachPersistentStore`. | n/a — always on once `EmergencyRepository.init(ctx)` runs. |
| uniffi case-vault cache | ⏳ Not yet wired (Stage 1 + Stage 2 of `BUILD.md`). The persistent queue above lives in plain SQLite for v0; the uniffi cache is the eventual home for it. | n/a |
| Push-wake (FCM) for incoming case-state events | ⏳ Not declared on the tablet side. The relay's push-wake path lands at `/v1/emergency/initiate`, but the tablet doesn't yet host an `FirebaseMessagingService`. | n/a |
| Foreground-service for active case | ⏳ `AndroidManifest.xml` has the permissions but no `<service>` declaration yet. Active case persists in memory only via `CaseVault`. | n/a |
| Sync indicator state machine | Partial — flips between `Synced` ↔ `Queued` based on list size; persistent queue feeds it. | Real worker emitting `Syncing` while flushing + `OfflineNoQueue` on transport-unreachable still TBD. |
| Real cert-chain validation on the patient response | ⏳ Tablet receives the relay-signed `EmergencyAccessRequest` payload but doesn't re-verify the chain. The patient phone does that (per `spec/emergency-trust.md`); tablet is the trusting end. The optional responder-cert layer (per-shift cert with the responder's identity baked in) is a v0.x deliverable. | n/a |
| Real Outfit / Inter / JetBrains Mono fonts | System fallback | Drop variable-font TTFs into `res/font/` + `Font(R.font.outfit, ...)` in `Type.kt`. Same path as `connect/android`. |

## UX choices made (and why)

These are decisions made because the spec is silent. Each is reversible
once feedback lands.

### Dark mode forced (no system-follow)

The Connect app uses dark-by-default-with-system-fallback. The Emergency
tablet **forces dark** regardless of system setting.

**Why.** Paramedic shifts span dawn-ambulance interiors (red/blue strobes)
and outdoor noon roadsides. Dark works in both. A user-toggle for light
mode would be a feature to forget on a chaotic shift; the dark surface
is the right default for every documented call scenario. If a deployment
needs light, that's a one-line theme override in `MainActivity`.

### Single-flow navigation, not a global tab bar

Connect has a 4-tab bottom-bar (Log / Dashboard / Grants / Settings).
The Emergency tablet uses **stacked navigation** at the app level (no
tabs above the case) and a **case-scoped tab bar** (Patient / Log /
Timeline / Handoff) inside an active case.

**Why.** Paramedics work one patient at a time. Tabs at the app level
would tempt drop-out (e.g. "let me check Settings mid-call"). The
case-scoped bar is fine because all four destinations are case-relevant.

### `screenOrientation=sensorLandscape` (landscape primary)

Tablet form factor is the primary target (10" landscape, dashboard /
chest-mount in the ambulance). Phone-portrait is a fallback.

**Why.** Two-column layout on the intervention screen needs landscape
real estate. Material3's `WindowSizeClass` would let us branch dynamically;
deferred to v0.x because the v0 demo is exercised on a tablet.

### Number pad over soft keyboard for vitals

The brief explicitly says no soft keyboard. We render a custom 3×4
button grid backed by `Modifier.aspectRatio(1.6f)` so each digit button
is glove-friendly.

**Why.** Soft keyboard on landscape tablets is small, easy to mistype,
and steals vertical space. A dedicated grid is faster for two-digit BP
entry and works through gloves. Drug name + observation / note still
use the soft keyboard because those are textual.

### Big-type ramp (+25% display / +15% body)

Material3's default type scale assumes phone-in-hand reading distance
(~30 cm). Tablet-on-stretcher / chest-mount distance is ~50–70 cm; we
add roughly +25% to display and +15% to body across the scale.

**Why.** Glance-readable from arm's length. Negotiable per real-world
testing with paramedics.

### Auto-grant indicator amber, not red

Per `screens-emergency.md` "Designer's handoff notes":
> The auto-granted badge needs a distinct visual treatment — different
> color (perhaps amber or muted red) …

We pick saturated amber (`EmergencyPalette.AutoGrant`) — distinct from
both urgent red and success green. Used on the break-glass resolution
chip, the patient-view header chip, and the timeline filter for the
"GrantOpened" entry.

### CaseVault: in-memory queue, not SQLite (v0)

Per `SPEC.md` "Trust boundary":
> Patient OHDC data — Cached in tablet RAM during active case;
> flushed on case close.

For the **patient data slice** that's the right shape. For the **queued
writes** an in-memory list is wrong long-term — a tablet reboot
mid-shift drops queued interventions. v0 keeps everything in-memory; the
data-extraction rule for `case_vault.db*` is already in place so the
SQLite table can land without further manifest churn.

### Fixed receiving-facility list of 3

`EmergencyRepository.knownReceivingFacilities()` returns a hard-coded
list of three Prague EDs.

**Why for v0.** Real deployment fetches the operator's "typical
destinations" config from the relay; that endpoint isn't pinned. Three
mock entries are enough to demo the picker affordance.

## What lands next (priority order)

1. **Canonical OHD beacon service UUID.** Replace
   `PLACEHOLDER_OHD_SERVICE_UUID` once `spec/emergency-trust.md` pins
   it. Connect-side (patient phone broadcaster) must agree.
2. **Relay-side `/v1/emergency/status/{request_id}` endpoint.** The
   tablet polls it; relay STATUS.md still has it under "What's stubbed
   / TBD". Until then the tablet falls through to a 5s mock auto-grant.
3. **Relay-side `/v1/emergency/handoff` endpoint.** Same — wire shape
   provisional in `OhdcClient.emergencyHandoff`. Once relay lands the
   endpoint, the tablet starts honouring the real successor case ULID
   without code changes.
4. **Real cert-chain handling end-to-end.** Tablet currently trusts the
   relay's signed `EmergencyAccessRequest` payload as opaque (the
   patient phone is the verifier per spec). The optional **per-shift
   responder cert** layer that ties the request to a specific
   paramedic isn't yet wired; the relay's `auth_mode` is on the
   relay side.
5. **uniffi case-vault cache.** Stage 1 + Stage 2 of `BUILD.md`. Will
   replace the plain SQLite `QueuedWriteStore` with the encrypted
   uniffi-backed cache + take the patient-data slice that's currently
   memory-only.
6. **Foreground service** for active-case cache + offline flush worker.
   Needs the FCM-side push-wake protocol on the tablet
   (`FirebaseMessagingService`) pinned. The relay's push-wake path
   lands at `/v1/emergency/initiate`.
7. **Push-wake for handoff completion.** Currently the tablet
   transitions to discovery on receipt of the handoff response; a
   push-wake from the receiving facility's relay (when the successor
   case opens) would let the tablet retire its read-only grant
   gracefully.
8. **Real recording UX for vitals number-pad → OHDC events.** The
   number-pad submission writes a single `vital.hr` / etc. channel; a
   richer "draft → review → commit" flow (let the paramedic batch
   several readings before committing) is a v0.x UX iteration.
9. **Sync indicator state machine.** `Syncing` + `OfflineNoQueue`
   states aren't yet emitted by a real worker.
10. **Silent OIDC refresh into the OHDC interceptor.**
    `AuthState.performActionWithFreshTokens` is wired in `OperatorSession`
    but not yet threaded into `OhdcClient`'s bearer provider.
11. **WindowSizeClass branching** for phone-portrait fallback layouts.
12. **Outfit / Inter / JetBrains Mono fonts** dropped into `res/font/`.
13. **iOS port** (Swift / SwiftUI). Deferred phase per SPEC.

## Smoke test (run-time)

NDK and Android SDK aren't in the scaffold environment; the smoke test
is documented but not executed. With both installed:

```bash
cd emergency/tablet

# (Optional, only when the case-vault uniffi cache is wired) Stage 1 + Stage 2:
cd ../../storage/crates/ohd-storage-bindings
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 \
  -o ../../../emergency/tablet/app/src/main/jniLibs build --release
cd ../..
cargo run --features cli --bin uniffi-bindgen -- generate \
  --library target/release/libohd_storage_bindings.so \
  --language kotlin \
  --out-dir emergency/tablet/app/src/main/java/uniffi

# Stage 3 — APK assemble. Pass the operator's relay URL so the OHDC
# client points at it (otherwise it falls through to the dev loopback).
cd emergency/tablet
./gradlew :app:assembleDebug \
  -Pohd.emergency.relay.base=https://relay.ems-prague.cz
./gradlew :app:installDebug

# Run unit tests (host JVM, no NDK / device required):
./gradlew :app:testDebugUnitTest
```

The unit tests cover:
- `OhdcClientTest` — Connect-Protocol unary + streaming round-trips,
  error envelope decoding, auth-header selection, the relay-private
  `/v1/emergency/initiate` + `/v1/emergency/handoff` endpoints.
- `EmergencyRepositoryTest` — the 7-step paramedic flow logic against
  a `MockWebServer` standing in for the relay (BreakGlass → Patient →
  Intervention → Timeline → Handoff). Exercises the OHDC commit /
  pending / error outcomes + the queued-write fallback.
- `CaseVaultLifecycleTest` — break-glass state-machine transitions +
  queued-writes lifecycle (enqueue, mark-flushed, clear).

Then on the device:

1. Sign in with the operator IdP (real OIDC) — or use the dev stub.
2. Tap "Scan for patients" — system permission dialog the first time;
   on grant, real BLE scan runs against the placeholder OHD service
   UUID. If no OHD beacons are advertising in range, falls back to
   manual entry. (On an emulator without BLE: the mock 3-beacon
   stream kicks in.)
3. Tap a beacon → "Send request" → real `/v1/emergency/initiate`
   POST + status poll until the patient approves. (When the relay
   doesn't yet expose `/v1/emergency/status/{request_id}`, the
   tablet falls through to a 5s mock auto-grant.)
4. Patient view loads from real OHDC `QueryEvents` under the case
   grant; falls back to mock data if no events under the grant.
5. Tap "Log" → pick "Heart rate" → enter "112" on the pad → Submit.
   The intervention POSTs via OHDC `PutEvents`. On success, the chip
   flips to "Synced"; on transport failure, it stays "Queued (1)"
   and the row carries the queued badge. The queued row survives a
   tablet reboot (persistent SQLite store).
6. Tap "Timeline" — events from OHDC are merged with the queued
   writes overlay.
7. Tap "Handoff" → "FN Motol — Emergency Department" → Confirm.
8. Returns to Discovery with the active-case banner cleared.

## Constraints respected

- Touched only files under `/home/jakub/contracts/personal/ohd/emergency/tablet/`.
- Did NOT touch `../spec/`, `../storage/`, `../relay/`, `../care/`,
  `../connect/`, `../emergency/dispatch/`, `../emergency/cli/`,
  `../emergency/mcp/`, `../emergency/deploy/`, `../emergency/spec/`.
- Did NOT git add / git commit.
- Did NOT run gradle / cargo-ndk (NDK not in env). All build commands
  documented in `BUILD.md`.
