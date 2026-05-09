# OHD Emergency Tablet — Android Build Recipe

End-to-end recipe for producing a runnable APK / AAB. Geared at developers
who already have an Android development environment installed; if you don't,
follow the **Prerequisites** section first.

The Emergency tablet app links the OHD Storage Rust core through uniffi.
Same crate as `connect/android`, **different deployment role**:

| `connect/android` | `emergency/tablet` |
|---|---|
| Primary on-device storage path. SQLCipher file holds the user's canonical OHD record. | **Local case-vault cache.** SQLCipher file holds the active-case snapshot + queued offline intervention writes. The authoritative store is the operator's relay-mediated remote storage; the tablet reflects. |
| `OhdStorage.create(...)` once at first run; `OhdStorage.open(...)` on every cold start. | `OhdStorage.create(...)` once at first run; `OhdStorage.open(...)` per-case (cleared on case close). |
| OHDC HTTP client used for sharing (Care/EMR), not for primary I/O. | OHDC HTTP client used for **everything that's not the local cache** — break-glass initiation, `PutEvents` for interventions, `QueryEvents` for the patient view, `HandoffCase`. |

So Stage 1 + Stage 2 of this recipe are identical to `connect/android/BUILD.md`
in mechanics — same Rust crate, same cargo-ndk invocation, same uniffi-bindgen
output. The OHDC client is a separate concern (lands as a Kotlin library
once the storage component publishes its first codegen drop; see
"OHDC client" below).

The build is **three-stage**:

1. **Stage 1 — Rust → JNI shared libraries.** Cross-compile the
   `ohd-storage-bindings` crate to one `libohd_storage_bindings.so` per
   Android ABI and drop them into `app/src/main/jniLibs/<abi>/`.
2. **Stage 2 — Kotlin bindings.** Generate the Kotlin façade for the
   uniffi surface, drop it into `app/src/main/java/uniffi/`.
3. **Stage 3 — Gradle assemble.** `./gradlew :app:assembleRelease` (or
   `assembleDebug`).

The app refuses to launch without Stage 1's `.so` files — JNA's
`Native.register(…)` throws `UnsatisfiedLinkError` when the cdylib is
missing.

> **Status note.** The version of this repo you're reading was scaffolded
> without the NDK installed; the `app/src/main/jniLibs/` and
> `app/src/main/java/uniffi/` directories are absent. **Run the steps below
> on first checkout.** None of them are run during scaffolding CI. The
> v0 scaffold's UI is exercisable end-to-end through `EmergencyRepository`'s
> stubbed call sites — Stage 1 + Stage 2 are required only when you wire
> the case-vault cache.

## Prerequisites

| Tool | Version | Why |
|---|---|---|
| Android Studio | Hedgehog (2023.1.1) or later | IDE + JDK + AVD manager |
| JDK | 17 | Matches `compileOptions { sourceCompatibility = VERSION_17 }` |
| Android SDK platforms | API 35 (target / compile), API 30 (min) | Per `app/build.gradle.kts` |
| Android NDK | r26+ (26.1.10909125 known good) | Required by `cargo ndk` |
| Android SDK build-tools | 34.0.0+ | AGP 8.6 floor |
| Android SDK platform-tools | latest | `adb` for installing |
| Rust toolchain | 1.88+ (workspace MSRV) | Per `storage/rust-toolchain.toml` |
| `cargo-ndk` | 3.5+ | Wraps `cargo build` with the NDK linker config |
| `rustup` Android targets | `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android` | Cross-compile targets |

The `minSdk` differs from `connect/android` (29 → 30) because the tablet
relies on `BLUETOOTH_SCAN`'s `neverForLocation` flag for paramedic-friendly
discovery without an "allow location" prompt. Older rugged tablets that
need legacy `BLUETOOTH` + `ACCESS_FINE_LOCATION` are still supported via
the `maxSdkVersion=30` permission entries in `AndroidManifest.xml`, but
the install floor itself is 30.

### Install the Android targets in rustup

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
```

### Install `cargo-ndk`

```bash
cargo install cargo-ndk
```

### Point `ANDROID_NDK_HOME` at the NDK

```bash
# macOS / Linux:
export ANDROID_NDK_HOME="$HOME/Library/Android/sdk/ndk/26.1.10909125"   # macOS
export ANDROID_NDK_HOME="$HOME/Android/Sdk/ndk/26.1.10909125"           # Linux

# Windows (PowerShell):
$env:ANDROID_NDK_HOME = "$env:LOCALAPPDATA\Android\Sdk\ndk\26.1.10909125"
```

Verify: `$ANDROID_NDK_HOME/source.properties` exists and prints
`Pkg.Revision=26.x.y`.

## Stage 1 — build the Rust core

From the repo root (containing `emergency/` and `storage/`):

```bash
cd storage/crates/ohd-storage-bindings
cargo ndk \
  -t arm64-v8a \
  -t armeabi-v7a \
  -t x86_64 \
  -o ../../../emergency/tablet/app/src/main/jniLibs \
  build --release
```

What this does:

- Compiles `ohd-storage-bindings` (and its dependency `ohd-storage-core`)
  for each listed ABI.
- Links each into `libohd_storage_bindings.so` using the NDK's
  `clang` + `lld` toolchain.
- Drops the resulting cdylibs into:

  ```
  emergency/tablet/app/src/main/jniLibs/arm64-v8a/libohd_storage_bindings.so
  emergency/tablet/app/src/main/jniLibs/armeabi-v7a/libohd_storage_bindings.so
  emergency/tablet/app/src/main/jniLibs/x86_64/libohd_storage_bindings.so
  ```

- AGP picks them up automatically and packages them into the APK's
  `lib/<abi>/` directory.

The first build will compile bundled SQLCipher (~3 minutes per ABI) because
`rusqlite/bundled-sqlcipher` is enabled in the workspace. Incremental builds
finish in seconds. Per-ABI release sizes (rough):

| ABI | Stripped `.so` | Notes |
|---|---|---|
| `arm64-v8a` | ~3 MB | Primary modern target. |
| `armeabi-v7a` | ~2.5 MB | Older devices; SQLCipher dominates. |
| `x86_64` | ~3.5 MB | Emulator. |

**Important:** Stage 1 is shared across `connect/android` and
`emergency/tablet`. The source-of-truth crate is the same; if you've
already built the `.so` files for `connect/android`, you can copy them
into `emergency/tablet/app/src/main/jniLibs/` instead of re-running
cargo-ndk. They're byte-identical.

## Stage 2 — generate Kotlin bindings

Once Stage 1 has produced an `.so` for at least one host-runnable ABI
(typical: the Linux/macOS `target/release/libohd_storage_bindings.so`
from a normal `cargo build`), run:

```bash
# From the repo root:
cd storage
cargo run --features cli --bin uniffi-bindgen -- \
  generate \
  --library target/release/libohd_storage_bindings.so \
  --language kotlin \
  --out-dir ../emergency/tablet/app/src/main/java/uniffi
```

Output:

```
emergency/tablet/app/src/main/java/uniffi/ohd_storage/ohd_storage.kt
```

This is the same Kotlin façade the connect/android module uses; the
metadata embedded in the cdylib is identical regardless of which Android
app you're building, so the generated package compiles unchanged.

> **Why a host `.so`, not the Android one?** uniffi-bindgen's `--library`
> mode reads metadata sections from any cdylib of the crate; the Android
> ABI cross-builds work too, but a host build is faster. The metadata is
> identical across architectures.

### What the generated Kotlin looks like

After codegen, Kotlin call sites look like:

```kotlin
import uniffi.ohd_storage.OhdStorage
import uniffi.ohd_storage.EventInputDto
import uniffi.ohd_storage.OhdException

val cache = try {
    OhdStorage.create(path = "/data/data/com.ohd.emergency/files/case_vault.db", keyHex = key)
} catch (e: OhdException.OpenFailed) { /* … */ }
```

uniffi's Kotlin codegen surfaces `OhdError` variants as
`OhdException.OpenFailed`, `OhdException.Auth`, `OhdException.InvalidInput`,
`OhdException.NotFound`, `OhdException.Internal` (sealed class).

The Compose layer never imports `uniffi.ohd_storage.*` directly — every
call site lives in `data/EmergencyRepository.kt`. Same pattern as
`connect/android`'s `data/StorageRepository.kt`.

## Stage 3 — Gradle assemble

```bash
cd emergency/tablet
./gradlew :app:assembleDebug          # → app/build/outputs/apk/debug/app-debug.apk
./gradlew :app:assembleRelease        # → app/build/outputs/apk/release/app-release-unsigned.apk
./gradlew :app:bundleRelease          # → app/build/outputs/bundle/release/app-release.aab
./gradlew :app:installDebug           # install to attached device / emulator
./gradlew :app:lint                   # lint pass
```

If Gradle complains it can't find the `uniffi` package, you forgot Stage 2.
If the app crashes on launch with `UnsatisfiedLinkError: dlopen failed:
library "libohd_storage_bindings.so" not found`, you forgot Stage 1 (or
forgot the ABI for your test device).

## OHDC client (landed 2026-05-09 — hand-rolled OkHttp + Connect-Protocol JSON)

Stages 1–3 cover the **local cache** path. The Emergency tablet's primary
wire is OHDC over HTTP/2 to the operator's relay-mediated remote storage.
v0 ships a hand-rolled Kotlin client at:

```
app/src/main/java/com/ohd/emergency/data/ohdc/
    OhdcClient.kt           — Connect-Protocol unary + server-streaming over OkHttp
    Dtos.kt                 — JSON DTOs mirroring `ohdc.v0` proto messages
    OhdcClientFactory.kt    — base-URL resolution + bearer wiring
```

### Why hand-rolled, not Connect-Kotlin

Two options were on the table:

| Option | Status |
|---|---|
| **A. Connect-Kotlin** ([connectrpc/connect-kotlin](https://github.com/connectrpc/connect-kotlin)): generated stubs from `buf gen`, AGP plugin, full proto-codegen pipeline. | Picked NOT for v0. |
| **B. Hand-rolled OkHttp + Connect-Protocol JSON**: small client speaking the unary/streaming wire by hand (POST + JSON body + `connect-protocol-version: 1` header). | **Picked for v0.** |

Reasons the hand-rolled path won:

1. The relay's `/v1/emergency/initiate` + `/v1/emergency/handoff` are
   **plain JSON over HTTP**, not Connect-RPC. Even with Connect-Kotlin
   we'd still need a hand-rolled HTTP path for those — doubling up on
   HTTP plumbing across two libraries adds friction.
2. Connect-Kotlin's `buf gen` pipeline + AGP plugin would force a
   proto-generation step into BUILD.md that the v0 demo doesn't need.
   The proto stubs aren't shipping from emergency/tablet's codegen yet.
3. The OHDC unary RPCs we need (WhoAmI, QueryEvents, PutEvents,
   GetCase, ListCases) are tiny payloads. Connect-Protocol JSON is
   specified at <https://connectrpc.com/docs/protocol/> — POST to
   `/<package>.<Service>/<Method>`, JSON body, JSON response.
4. Connect-Web (the sibling Care app at `care/web/src/ohdc/client.ts`)
   speaks the same wire; this client mirrors that pattern in Kotlin.
5. Streaming RPCs (QueryEvents returns `stream Event`) are handled by
   Connect-Protocol's enveloped-stream framing
   (`[1 byte flags][4 bytes BE length][N JSON bytes]`).

Once the storage component publishes binary-protobuf Kotlin codegen,
the surface gains a sibling `OhdcBinaryClient`. The high-level
[`EmergencyRepository`] API does not change.

### Dependencies added

```kotlin
implementation("com.squareup.okhttp3:okhttp:4.12.0")
testImplementation("com.squareup.okhttp3:mockwebserver:4.12.0")
testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.9.0")
```

### Configuration

The OHDC client resolves its base URL in this order:

1. `OperatorSession.relayBaseUrl(ctx)` — set by an out-of-band
   onboarding flow (QR-onboarding at shift-in, MDM-pushed config).
2. `BuildConfig.OHD_EMERGENCY_RELAY_BASE` — built into the APK.
   Override at build time:

   ```bash
   ./gradlew :app:assembleDebug \
     -Pohd.emergency.relay.base=https://relay.ems-prague.cz
   ```

3. Hard-coded dev fallback `http://10.0.2.2:8443` — the Android-emulator
   loopback to a relay running on the developer's host machine.

### Bearer selection

| Endpoint class | Bearer | Setter |
|---|---|---|
| Relay-private REST (`/v1/emergency/initiate`, `/v1/emergency/handoff`, `/v1/auth/info`) | Operator OIDC bearer | `OperatorSession.bearer(ctx)` (set by the AppAuth flow) |
| OHDC unary + streaming (`ohdc.v0.OhdcService/...`) | Per-case grant token | `CaseVault.activeCase.value.grantToken` (set by break-glass approval) |

The OHDC client picks the right bearer based on the `useGrantToken`
flag inside each method; callers don't choose.

## Real BLE scan (landed 2026-05-09)

`data/BleScanner.kt` ships two implementations:

- `MockBleScanner` — emits 3 mock patients in ~3s (used when
  `BLUETOOTH_SCAN` runtime permission is missing or BLE hardware is
  absent — e.g. the Android emulator without a host BT adapter).
- `RealBleScanner` — uses Android's `BluetoothLeScanner` against the
  OHD service UUID; parses service-data for the rotating beacon ID.

`EmergencyRepository.bleScanner()` selects between them at call time
based on `hasBleScanPermission(ctx)`.

### OHD beacon service UUID — placeholder

The canonical OHD beacon service UUID is **deferred to v0.x** per
`spec/emergency-trust.md` "Open items". v0 uses a placeholder:

```kotlin
const val PLACEHOLDER_OHD_SERVICE_UUID = "0000FED0-0000-1000-8000-00805F9B34FB"
```

When the canonical UUID lands, replace this constant in
`data/BleScanner.kt`. The real client side (Connect-Android beacon
broadcaster, also TBD) must broadcast under the same UUID. No other
code change required on the tablet.

### Beacon-ID format (provisional)

Service-data payload under the OHD service UUID:

```
[0..16]   rotating opaque beacon ID (16 bytes)
[16..]    reserved
```

If service-data is missing or shorter than 16 bytes, [`RealBleScanner`]
falls back to `result.device.address` as a stable-but-not-canonical
identifier so the row still renders (degraded mode — paramedic uses
manual entry).

### Permissions (already in `AndroidManifest.xml`)

| Permission | Usage |
|---|---|
| `BLUETOOTH_SCAN` (API 31+) with `usesPermissionFlags="neverForLocation"` | Modern BLE scan without location prompt |
| `ACCESS_FINE_LOCATION` (≤ API 30) | Pre-Android 12 BLE-implies-location |

The `DiscoveryScreen` requests the appropriate permission via
`rememberLauncherForActivityResult` on first "Scan for patients" tap;
`hasBleScanPermission(ctx)` short-circuits the launcher when the
permission is already granted.

## Real `/v1/emergency/initiate` (landed 2026-05-09)

`EmergencyRepository.initiateBreakGlass(...)` now POSTs to the relay's
`/v1/emergency/initiate` endpoint (mirrors
`relay/src/server.rs::handle_emergency_initiate`):

```
POST {operator_relay}/v1/emergency/initiate
Authorization: Bearer <operator OIDC bearer>
Content-Type: application/json
{
  "rendezvous_id": "<patient beacon ID>",
  "responder_label": "Officer Novák",
  "operator_label":  "EMS Prague Region",
  "scene_context":   "Václavské nám.",
  ...
}
→ 200
{ "signed_request": { "request_id": "...", ... },
  "delivery_status": "delivered" | "pushed" | "no_token" }
```

After the initiate POST, the tablet polls
`/v1/emergency/status/{request_id}` (relay endpoint TBD — see relay
STATUS.md) once per second until a terminal state surfaces:

- `approved` / `auto_granted` — tablet receives the case ULID +
  grant token and navigates to the patient view.
- `rejected` — tablet shows the rejection chip + back-to-discovery.
- `timed_out` — tablet shows the timeout chip.

When the status endpoint returns 404 (relay hasn't shipped it yet),
the tablet falls back to a 5s mock auto-grant so the v0 demo flow
still works end-to-end.

## Persistent offline-write queue (landed 2026-05-09)

`data/QueuedWriteStore.kt` backs `CaseVault.queuedWrites` with a
plain SQLite table at `databases/case_vault.db`. Schema:

```sql
CREATE TABLE queued_writes (
  local_ulid    TEXT PRIMARY KEY NOT NULL,
  case_ulid     TEXT NOT NULL,
  occurred_ms   INTEGER NOT NULL,
  recorded_ms   INTEGER NOT NULL,
  kind          TEXT NOT NULL,
  summary       TEXT NOT NULL,
  payload_json  TEXT NOT NULL
);
CREATE INDEX idx_queued_case ON queued_writes(case_ulid, recorded_ms);
```

A tablet reboot mid-shift no longer drops queued intervention writes —
on app restart `CaseVault.attachPersistentStore(...)` loads any unflushed
rows back into the in-memory queue and the OHDC flush worker can drain
them. The `case_vault.db*` files are already excluded from auto-backup
via `data_extraction_rules.xml`.

Why hand-rolled SQLite (not Room): the schema is one table; Room would
pull KSP + a code-generation pass into the build. SQLiteOpenHelper is
~80 lines of straightforward boilerplate.

## File layout after a complete build

```
emergency/tablet/
├── BUILD.md                                       ← this file
├── README.md
├── STATUS.md
├── settings.gradle.kts
├── build.gradle.kts
├── gradle.properties
├── gradle/libs.versions.toml
└── app/
    ├── build.gradle.kts
    ├── proguard-rules.pro
    └── src/main/
        ├── AndroidManifest.xml
        ├── java/
        │   ├── com/ohd/emergency/                 ← hand-written Kotlin app code
        │   │   ├── MainActivity.kt
        │   │   ├── Routes.kt
        │   │   ├── data/
        │   │   │   ├── EmergencyRepository.kt
        │   │   │   ├── CaseVault.kt
        │   │   │   ├── BleScanner.kt
        │   │   │   ├── MockPatientData.kt
        │   │   │   └── OperatorSession.kt
        │   │   └── ui/
        │   │       ├── theme/
        │   │       │   ├── Color.kt
        │   │       │   ├── Type.kt
        │   │       │   └── Theme.kt
        │   │       ├── components/
        │   │       │   ├── TopBar.kt
        │   │       │   ├── SyncIndicator.kt
        │   │       │   ├── StatusChip.kt
        │   │       │   ├── CriticalCard.kt
        │   │       │   ├── QuickEntry.kt
        │   │       │   └── VitalsPad.kt
        │   │       └── screens/
        │   │           ├── LoginScreen.kt
        │   │           ├── DiscoveryScreen.kt
        │   │           ├── BreakGlassScreen.kt
        │   │           ├── PatientScreen.kt
        │   │           ├── InterventionScreen.kt
        │   │           ├── TimelineScreen.kt
        │   │           ├── HandoffScreen.kt
        │   │           └── CaseNavBar.kt
        │   └── uniffi/                            ← generated, gitignored
        │       └── ohd_storage/
        │           └── ohd_storage.kt             ← from Stage 2
        ├── jniLibs/                               ← generated, gitignored
        │   ├── arm64-v8a/libohd_storage_bindings.so
        │   ├── armeabi-v7a/libohd_storage_bindings.so
        │   └── x86_64/libohd_storage_bindings.so
        └── res/
            ├── values/strings.xml
            └── xml/data_extraction_rules.xml
```

## Version pins

| Component | Pinned to | Where |
|---|---|---|
| Android Gradle Plugin | 8.6.1 | `gradle/libs.versions.toml` (`agp`) |
| Kotlin | 2.0.21 | `gradle/libs.versions.toml` (`kotlin`) |
| Compose BOM | 2024.10.01 | `gradle/libs.versions.toml` (`composeBom`) |
| Navigation Compose | 2.8.4 | `gradle/libs.versions.toml` |
| JNA | 5.14.0 | `gradle/libs.versions.toml` (`jna`) — uniffi runtime needs ≥5.13 |
| uniffi (Rust) | 0.28 | `storage/crates/ohd-storage-bindings/Cargo.toml` |
| NDK | r26+ (26.1+) | environment, see Prerequisites |
| min SDK | 30 (Android 11) | `app/build.gradle.kts` |
| target SDK | 35 (Android 15) | `app/build.gradle.kts` |
| compile SDK | 35 | `app/build.gradle.kts` |
| Java | 17 | `compileOptions` |

## Running on a tablet device

The app declares `screenOrientation=sensorLandscape` in the manifest;
phone portrait is rendered for testing only and isn't a primary target.
Recommended hardware:

| Class | Example | Notes |
|---|---|---|
| Rugged paramedic tablet | Samsung Galaxy Tab Active 5 | The reference-deployment shape. Sunlight-readable; gloves-friendly. |
| Generic 10" Android tablet | Pixel Tablet | Fine for development. |
| Phone | Pixel 7+ | Phone-portrait works but several screens scroll horizontally; not the primary target. |

## CI considerations

A future GitHub Actions workflow runs Stages 1 + 2 + 3 on the
`ubuntu-latest` runner. Until then, the `.so` files and the generated
`uniffi/ohd_storage.kt` are **not committed** — `.gitignore` excludes
both `app/src/main/jniLibs/` and `app/src/main/java/uniffi/`. Each
contributor regenerates locally.

The Gradle helper task `tasks.register("buildRustCore")` in
`app/build.gradle.kts` documents the cargo-ndk command but **does not
execute it** in this scaffolding phase — same reasoning as
`connect/android/BUILD.md`.

## Troubleshooting

The error matrix is identical to `connect/android/BUILD.md`:

- `cargo ndk` errors with "linker not found" → `$ANDROID_NDK_HOME`
  unset / stale.
- `assembleDebug` complains about minSdk → check `defaultConfig.minSdk = 30`.
- `UnsatisfiedLinkError` at runtime → Stage 1 didn't run, or you
  installed an APK whose ABI list doesn't include your test device.
- `NoClassDefFoundError: com.sun.jna.Native` → `@aar` suffix missing
  on the JNA dependency. Verify
  `app/build.gradle.kts` ends the JNA line with `@aar`.
- R8 strips uniffi callbacks in release → keep rules already in
  `app/proguard-rules.pro`. Verify they survived a manual edit.

## Cross-references

- Sibling Android app: [`../../connect/android/BUILD.md`](../../connect/android/BUILD.md)
- uniffi crate: [`../../storage/crates/ohd-storage-bindings/`](../../storage/crates/ohd-storage-bindings/)
- uniffi surface (Rust): [`../../storage/crates/ohd-storage-bindings/src/lib.rs`](../../storage/crates/ohd-storage-bindings/src/lib.rs)
- App architecture rationale: [`../SPEC.md`](../SPEC.md) "Form factors → Android"
- Visual design: [`../../ux-design.md`](../../ux-design.md)
- Screen flow: [`../spec/screens-emergency.md`](../spec/screens-emergency.md)

## Operator OIDC (AppAuth-Android) — wired 2026-05-09

The operator-OIDC sign-in flow lives in
[`app/src/main/java/com/ohd/emergency/data/OidcManager.kt`](app/src/main/java/com/ohd/emergency/data/OidcManager.kt)
and is wired to [`LoginScreen`](app/src/main/java/com/ohd/emergency/ui/screens/LoginScreen.kt)
via `rememberLauncherForActivityResult`. The flow is **independent of
Stages 1–3** above — it doesn't touch the Rust core or uniffi codegen
at all. So it can be exercised on a stock Android emulator with just
`./gradlew :app:assembleDebug` (modulo the cargo-ndk dependency which
makes the rest of the app actually run).

### Library

Pinned in `app/build.gradle.kts`:

```kotlin
implementation("net.openid:appauth:0.11.1")
implementation("androidx.security:security-crypto:1.1.0-alpha06")
```

`AppAuth-Android` ships its own `RedirectUriReceiverActivity` in its
library manifest. AGP merges it into your final APK manifest based on
the `appAuthRedirectScheme` placeholder, so you don't need to declare
the receiver yourself. The placeholder is set to `com.ohd.emergency`
in `defaultConfig.manifestPlaceholders`.

### Configuration

Three values feed the LoginScreen's defaults — set per deployment by
passing Gradle properties (`-P`) or editing
[`app/build.gradle.kts`](app/build.gradle.kts) directly:

| Property | Default | Purpose |
|---|---|---|
| `ohd.emergency.oidc.issuer` | `https://idp.example.cz/realms/ems` | Operator IdP issuer URL. AppAuth hits `/.well-known/openid-configuration` under it. |
| `ohd.emergency.oidc.client_id` | `ohd-emergency-tablet` | OAuth client_id registered with the IdP. |
| `ohd.emergency.oidc.redirect` | `com.ohd.emergency:/oidc-callback` | Custom-scheme redirect URI. **Must** match the registered redirect at the IdP and the `appAuthRedirectScheme` manifest placeholder. |

Override at build time:

```bash
./gradlew :app:assembleDebug \
  -Pohd.emergency.oidc.issuer=https://idp.ems-prague.cz/realms/dispatch \
  -Pohd.emergency.oidc.client_id=ohd-emergency-tablet \
  -Pohd.emergency.oidc.redirect=com.ohd.emergency:/oidc-callback
```

### Token storage

Tokens land in `EncryptedSharedPreferences` (`androidx.security:security-crypto`),
backed by a Keystore-bound AES-256-GCM master key. The
[`OperatorSession`](app/src/main/java/com/ohd/emergency/data/OperatorSession.kt)
singleton is the only call site; if Keystore is unavailable (corrupted
emulator, etc.) it transparently falls back to plain SharedPreferences
under the legacy `ohd_emergency_state` name and logs a warning. The
contract on `OperatorSession.bearer` etc. is unchanged.

### Flow at runtime

1. `LoginScreen` reads `BuildConfig.OHD_EMERGENCY_OIDC_*` defaults.
2. The user taps **Sign in with operator IdP**. `OidcManager.startAuthFlow`
   calls `AuthorizationServiceConfiguration.fetchFromIssuer(issuer)` to
   discover the AS metadata, builds an `AuthorizationRequest` with PKCE,
   and launches the IdP in a Custom Tab via the registered
   `ActivityResultLauncher<Intent>`.
3. The user authenticates with the IdP. The IdP redirects to
   `com.ohd.emergency:/oidc-callback?code=…&state=…` which AppAuth's
   bundled `RedirectUriReceiverActivity` intercepts and bounces back
   into the app's `Activity` via the result launcher.
4. `OidcManager.handleAuthResult` exchanges the code for tokens via
   `AuthorizationService.performTokenRequest` (PKCE verifier sent
   alongside), decodes the id_token claims (best-effort, unsigned —
   we use them only for display), and persists the bearer + refresh +
   AppAuth state JSON to `OperatorSession`.
5. `LoginScreen`'s `onSignedIn` callback navigates to `/discovery`.

### Smoke test

```bash
# Stock emulator on Android 13+ (no NDK / cargo-ndk needed for this leg):
./gradlew :app:assembleDebug
adb install app/build/outputs/apk/debug/app-debug.apk

# Pick an IdP that has the device's redirect URI pre-registered.
# A throwaway Keycloak / dex setup is sufficient.
```

### Known gaps

- **Silent refresh**: AppAuth's `AuthState.performActionWithFreshTokens`
  isn't yet wired into the OHDC HTTP client. The persisted state JSON
  is ready (`OperatorSession.appAuthStateJson`); pickup work is to
  thread it into `EmergencyRepository`'s OHDC interceptor when
  `accessExpiresAtMs` < 60s out.
- **Sign-out RP-initiated logout**: the IdP's `end_session_endpoint`
  is not called on panic-logout. The local clear is in
  `OperatorSession.signOut(ctx)`; adding the upstream call requires
  pulling `AuthorizationServiceConfiguration` back out of the
  persisted `AuthState`.
