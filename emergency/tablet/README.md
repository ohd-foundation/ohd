# `tablet/` — OHD Emergency Paramedic Tablet (Android)

> Android (Kotlin + Jetpack Compose) app for the paramedic tablet — the
> ambulance-form-factor side of the OHD Emergency component. iOS port
> deferred.

## What's here

A Compose-based Android app with the full break-glass flow wired
end-to-end:

- **Login** — operator-OIDC via AppAuth-Android against the operator's IdP. Tokens encrypted at rest via Keystore-bound `EncryptedSharedPreferences`. See [`BUILD.md`](BUILD.md) "Operator OIDC".
- **Discovery** — real BLE scan (`BluetoothLeScanner` against the OHD service UUID; falls back to mock when the host has no BT adapter or the runtime permission is missing) + manual-entry fallback. Active-case banner if a case is in flight.
- **Break-glass** — POST to relay's `/v1/emergency/initiate`, then poll `/v1/emergency/status/{request_id}`. "Auto-granted via timeout" / "Approved" / "Rejected" / "Timed out" terminal states. 5s mock fallback when the relay's status endpoint isn't deployed yet.
- **Patient view** — red-bordered critical-info card (allergies, blood type, advance directives), active medications, recent vitals (5 cards with sparklines), active diagnoses, recent observations. A "hide non-emergency data" toggle in the header.
- **Intervention logging** — quick-entry pads for HR / BP / SpO2 / Temp / Drug / Observation / Note. Vitals use a chunky number pad (not the soft keyboard); BP is two-field (systolic + diastolic); drug is name + dose + unit + route; observation / note are free-text.
- **Timeline** — chronological case feed with filter chips (All / Vitals / Drugs / Observations / System). Queued-but-not-flushed writes flagged. The queue persists across reboots via SQLite (`databases/case_vault.db`); see [`BUILD.md`](BUILD.md) "Persistent offline-write queue".
- **Handoff** — receiving-facility picker + manual-entry field + optional summary note. On confirm, returns to discovery.

The Compose tree never imports `uniffi.ohd_storage.*` directly; every call site lives in `data/EmergencyRepository.kt`. That keeps the rest of the app compiling even when the BUILD.md Stage 1 / Stage 2 codegen hasn't been run yet.

The OHDC client is hand-rolled OkHttp + Connect-Protocol JSON; rationale and shape in [`BUILD.md`](BUILD.md) "OHDC client".

## Layout

```
tablet/
├── settings.gradle.kts
├── build.gradle.kts
├── gradle.properties
├── gradle/libs.versions.toml          ← version catalogue
├── BUILD.md                           ← three-stage build recipe
├── README.md                          ← this file
├── STATUS.md                          ← what's wired vs mocked
└── app/
    ├── build.gradle.kts
    ├── proguard-rules.pro
    └── src/main/
        ├── AndroidManifest.xml        ← BLE / FG-service / wake-lock perms
        └── java/com/ohd/emergency/
            ├── MainActivity.kt        ← single-activity NavHost
            ├── Routes.kt              ← /login → /discovery → … → /handoff
            ├── data/
            │   ├── EmergencyRepository.kt   ← OHDC + relay facade (stubbed)
            │   ├── CaseVault.kt             ← in-memory case state machine
            │   ├── BleScanner.kt            ← mock BLE scan results
            │   ├── MockPatientData.kt       ← hand-rolled patient profile
            │   └── OperatorSession.kt       ← OIDC bearer storage (stub)
            └── ui/
                ├── theme/{Color,Type,Theme}.kt
                ├── components/
                │   ├── TopBar.kt
                │   ├── SyncIndicator.kt
                │   ├── StatusChip.kt
                │   ├── CriticalCard.kt
                │   ├── QuickEntry.kt
                │   └── VitalsPad.kt          ← chunky number pad
                └── screens/
                    ├── LoginScreen.kt
                    ├── DiscoveryScreen.kt
                    ├── BreakGlassScreen.kt
                    ├── PatientScreen.kt
                    ├── InterventionScreen.kt
                    ├── TimelineScreen.kt
                    ├── HandoffScreen.kt
                    └── CaseNavBar.kt
```

## Smoke test

**NOT RUN at scaffold time** — NDK isn't in this environment. To run it:

1. Follow the three stages in [`BUILD.md`](BUILD.md).
2. `./gradlew :app:installDebug` against an Android 11+ tablet (or
   emulator).
3. Sign in (any non-empty values; v0 stub).
4. Tap "Scan for patients" — three mock beacons arrive in ~3 seconds.
5. Tap a beacon → "Send request" → 5-second countdown → auto-grant.
6. Patient view loads with mock data; tap **Log** to record an
   intervention; tap **Timeline** to see it appear.
7. Tap **Handoff** → pick a receiving facility → confirm → returns to
   discovery.

## UX direction

Per the brief and `ux-design.md`:

- **Dark theme only.** Paramedic shifts span 04:00 ambulance interiors
  and noon-sun roadsides; a dark surface is readable in both. The
  Connect app uses dark-by-default-with-light-fallback; Emergency
  forces dark.
- **Big targets.** All primary buttons are 64–72 dp tall; quick-entry
  cards have generous internal padding so gloves and adrenaline don't
  miss the tap.
- **Red accents.** Brand red on the primary action (break-glass send,
  intervention submit), the active-case banner dot, and the critical-
  info card border. Amber is reserved for the auto-granted indicator
  per the designer's-handoff note in `screens-emergency.md`.
- **Big type.** The display / headline / body scale is bumped ~25% over
  the Material3 default to account for chest-mount-tablet reading
  distance (~50–70 cm vs phone-in-hand ~30 cm).
- **Single-flow navigation.** No bottom-tab bar at the app level — a
  paramedic moves linearly through one patient at a time. The bottom
  nav inside a case (Patient / Log / Timeline / Handoff) is
  case-scoped.
- **Number pad over soft keyboard.** Vitals entry uses a custom 3×4
  grid; the soft keyboard is reserved for textual fields.

## Auth model (intended)

Per [`../SPEC.md`](../SPEC.md) "Auth model on the tablet":

- Operator OIDC bearer at shift-in. v0 stub stores in plain
  SharedPreferences; v1 uses `androidx.security:security-crypto`'s
  EncryptedSharedPreferences with a Keystore-bound MasterKey.
- Optional per-shift responder cert (1–4h validity), private key in
  Android Keystore secure element.
- Active case grant tokens — memory only via [CaseVault], never disk.

## BLE

`BLUETOOTH_SCAN` declared with `usesPermissionFlags="neverForLocation"`
(Android 12+) so the OS doesn't infer location from BLE scans.
Pre-Android-12 devices fall back to legacy `BLUETOOTH` +
`ACCESS_FINE_LOCATION` (capped at `maxSdkVersion=30`).

Concrete BLE service UUID + characteristic IDs are TBD — open item in
[`../spec/emergency-trust.md`](../spec/emergency-trust.md) "Open items".
The mock scanner emits three patients with realistic RSSI / distance
shapes so the discovery UX can be exercised before the protocol pins
the wire.

## What lands next

See [`STATUS.md`](STATUS.md) for the complete scoreboard. Open items:

1. **OHDC Kotlin binary client.** A sibling typed binary-protobuf client may replace the hand-rolled JSON one once storage publishes a Kotlin codegen drop. The `EmergencyRepository` facade shields the Compose tree from the swap.
2. **Canonical OHD beacon service UUID.** v0 ships a placeholder ([`BUILD.md`](BUILD.md) "OHD beacon service UUID — placeholder"); pin in `spec/emergency-trust.md` and replace the constant.
3. **uniffi case-vault cache.** Stage 1 + Stage 2 of [`BUILD.md`](BUILD.md) so the local case snapshot is encrypted-at-rest via SQLCipher.
4. **iOS port.** Deferred until the Android app stabilizes.

## Why Android first

Per [`../SPEC.md`](../SPEC.md): paramedic tablets in Europe and most
low-cost EMS deployments are predominantly Android (rugged Samsung
Active tablets, Lenovo medical-grade hardware). iOS is added once the
Android app stabilizes.

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root.
