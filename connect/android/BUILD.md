# OHD Connect — Android Build Recipe

End-to-end recipe for producing a runnable APK / AAB. Geared at developers
who already have an Android development environment installed; if you don't,
follow the **Prerequisites** section first.

> The Stage 1 (cargo-ndk) and Stage 2 (uniffi-bindgen) commands are
> functionally identical to [`emergency/tablet/BUILD.md`](../../emergency/tablet/BUILD.md)
> — both apps consume the same `ohd-storage-bindings` crate. If you've
> built the `.so` files for one, you can copy them into the other's
> `app/src/main/jniLibs/`. The Kotlin facade is byte-identical too.

The Android app links the OHD Storage Rust core through uniffi. That means
the build is **two-stage**:

1. **Stage 1 — Rust → JNI shared libraries.** Cross-compile the
   `ohd-storage-bindings` crate to one `libohd_storage_bindings.so` per
   Android ABI and drop them into `app/src/main/jniLibs/<abi>/`.
2. **Stage 2 — Kotlin bindings + Gradle assemble.** Generate the Kotlin
   façade for the uniffi surface, drop it into `app/src/main/java/uniffi/`,
   then run `./gradlew assembleRelease` (or `assembleDebug`).

The app refuses to launch without Stage 1's `.so` files — JNA's
`Native.register(…)` throws `UnsatisfiedLinkError` when the cdylib is
missing.

> **Status note.** The version of this repo you're reading was scaffolded
> without the NDK installed; the `app/src/main/jniLibs/` and
> `app/src/main/java/uniffi/` directories ship empty. **Run the steps below
> on first checkout.** None of them are run during scaffolding CI.

## Prerequisites

| Tool | Version | Why |
|---|---|---|
| Android Studio | Hedgehog (2023.1.1) or later | IDE + JDK + AVD manager |
| JDK | 17 | Matches `compileOptions { sourceCompatibility = VERSION_17 }` |
| Android SDK platforms | API 34 (target), API 29 (min) | Per `app/build.gradle.kts` |
| Android NDK | r26+ (26.1.10909125 known good) | Required by `cargo ndk` |
| Android SDK build-tools | 34.0.0+ | AGP requirement |
| Android SDK platform-tools | latest | `adb` for installing |
| Rust toolchain | 1.88+ (workspace MSRV) | Per `storage/rust-toolchain.toml` |
| `cargo-ndk` | 3.5+ | Wraps `cargo build` with the NDK linker config |
| `rustup` Android targets | `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android` | Cross-compile targets |

### Install the Android targets in rustup

```bash
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
```

### Install `cargo-ndk`

```bash
cargo install cargo-ndk
```

### Point `ANDROID_NDK_HOME` at the NDK

`cargo-ndk` needs to know where the NDK lives. With Android Studio's SDK
manager:

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

From the repo root (containing `connect/` and `storage/`):

```bash
cd storage/crates/ohd-storage-bindings
cargo ndk \
  -t arm64-v8a \
  -t armeabi-v7a \
  -o ../../../connect/android/app/src/main/jniLibs \
  build --release
```

> **`x86_64` is intentionally dropped.** Building the vendored OpenSSL for
> `x86_64-linux-android` with NDK r26 fails — clang's integrated assembler
> rejects the modern SHA-NI / SM3 SIMD mnemonics in `crypto/sm3/sm3-x86_64.S`
> (`invalid instruction mnemonic 'vsm3rnds2'`). Real devices are arm64; this
> target is emulator-only. Re-add when (a) the NDK ships a newer clang or
> (b) the openssl-src crate disables those asm files for android-x86_64.

> **Why vendored OpenSSL?** The workspace `Cargo.toml` enables
> `rusqlite/bundled-sqlcipher-vendored-openssl` (not just `bundled-sqlcipher`)
> so SQLCipher's OpenSSL dependency is built from source instead of looked
> up via system headers. The NDK doesn't ship `openssl/crypto.h`, and we
> don't want to pin to a per-host openssl install.

What this does:

- Compiles `ohd-storage-bindings` (and its dependency `ohd-storage-core`)
  for each listed ABI.
- Links each into `libohd_storage_bindings.so` using the NDK's
  `clang` + `lld` toolchain (cargo-ndk auto-discovers `$ANDROID_NDK_HOME`
  and derives the right linker per target).
- Drops the resulting cdylibs into:

  ```
  connect/android/app/src/main/jniLibs/arm64-v8a/libohd_storage_bindings.so
  connect/android/app/src/main/jniLibs/armeabi-v7a/libohd_storage_bindings.so
  connect/android/app/src/main/jniLibs/x86_64/libohd_storage_bindings.so
  ```

- AGP picks them up automatically and packages them into the APK's
  `lib/<abi>/` directory.

If you also want emulator-on-Apple-Silicon support, add `-t arm64-v8a`
(host-arch agnostic — the Android emulator on M1/M2/M3 macs runs
`arm64-v8a`, while the older x86_64 emulator path needs the third target).

The first build will compile bundled SQLCipher (~3 minutes per ABI) because
`rusqlite/bundled-sqlcipher` is enabled in the workspace. Incremental builds
finish in seconds.

### Per-ABI release sizes (rough)

| ABI | Stripped `.so` | Notes |
|---|---|---|
| `arm64-v8a` | ~3 MB | Primary modern target. |
| `armeabi-v7a` | ~2.5 MB | Older devices; SQLCipher is the dominant cost. |
| `x86_64` | ~3.5 MB | Emulator-only; ship in debug builds, omit from Play release if you want. |

## Stage 2 — generate Kotlin bindings

Build a host-platform `.so` **in debug mode** so the symbol table is
preserved (uniffi 0.28's library-mode bindgen reads `.symtab`, which
`cargo build --release` strips — the bindgen exits 0 silently with no
output if it can't find the symbols):

```bash
# From the repo root:
cd storage
cargo build -p ohd-storage-bindings        # debug, symtab intact
cargo run --features cli --bin uniffi-bindgen -- \
  generate \
  --library --language kotlin \
  --out-dir ../connect/android/app/src/main/java/uniffi \
  target/debug/libohd_storage_bindings.so
```

Output:

```
connect/android/app/src/main/java/uniffi/ohd_storage/ohd_storage.kt
```

> **uniffi 0.28 + Kotlin 2.0 patch.** The generated file declares
> `val \`message\`: kotlin.String` in each `OhdException` variant constructor
> (a property), and ALSO an `override val message get() = "…"` block —
> Kotlin 2.0's stricter overload-resolution rejects the duplicate. Fix:
> add `override` to the constructor val and drop the formatter `get()`:
>
> ```bash
> sed -i 's/val \`message\`: kotlin\.String/override val \`message\`: kotlin.String/g' \
>   ../connect/android/app/src/main/java/uniffi/ohd_storage/ohd_storage.kt
> # then strip the per-variant `override val message  get() = "…"` blocks.
> ```
>
> A future bump to uniffi 0.29+ should obviate this — track upstream issue
> for "uniffi error variants collide with Throwable.message under Kotlin 2".

> **The bindgen output is placed at `<out-dir>/uniffi/ohd_storage/ohd_storage.kt`,
> not directly at `<out-dir>/ohd_storage/...`.** uniffi prepends a redundant
> `uniffi/` directory; either move it up one level or accept the deeper path
> (Kotlin packages are decided by the `package` declaration, not by directory
> layout, so AGP compiles it either way).

This is the Kotlin façade — every `OhdStorage` method, `EventInputDto`,
`PutEventOutcomeDto`, and `OhdError` variant from
`storage/crates/ohd-storage-bindings/src/lib.rs` becomes a class / data
class / sealed exception in `package uniffi.ohd_storage`.

> **Why a host `.so`, not the Android one?** uniffi-bindgen's `--library`
> mode reads metadata sections from any cdylib of the crate; the Android
> ABI cross-builds work too, but a host build is faster. The metadata is
> identical across architectures.

### What the generated Kotlin looks like

After codegen, Kotlin call sites look like:

```kotlin
import uniffi.ohd_storage.OhdStorage
import uniffi.ohd_storage.EventInputDto
import uniffi.ohd_storage.ChannelValueDto
import uniffi.ohd_storage.ValueKind
import uniffi.ohd_storage.OhdException

val storage = try {
    OhdStorage.create(path = "/data/data/com.ohd.connect/files/data.db", keyHex = key)
} catch (e: OhdException.OpenFailed) { /* ... */ }
```

uniffi's Kotlin codegen surfaces `OhdError` variants as
`OhdException.OpenFailed`, `OhdException.Auth`, `OhdException.InvalidInput`,
`OhdException.NotFound`, `OhdException.Internal` (sealed class).

## Stage 3 — Gradle assemble

> **First-time only — generate the gradle wrapper.** `gradlew` and
> `gradle/wrapper/` are not committed today; bootstrap them with a
> system-installed gradle:
>
> ```bash
> cd connect/android
> gradle wrapper --gradle-version 8.7
> ```
>
> Subsequent runs use `./gradlew` directly. `ANDROID_HOME` /
> `ANDROID_SDK_ROOT` must point at the SDK (e.g. `/opt/android-sdk`).

```bash
cd connect/android
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

## File layout after a complete build

```
connect/android/
├── BUILD.md                                      ← this file
├── README.md
├── settings.gradle.kts
├── build.gradle.kts
├── gradle.properties
└── app/
    ├── build.gradle.kts
    ├── proguard-rules.pro
    └── src/main/
        ├── AndroidManifest.xml
        ├── java/
        │   ├── com/ohd/connect/                  ← hand-written Kotlin app code
        │   │   ├── MainActivity.kt
        │   │   ├── data/
        │   │   │   ├── StorageRepository.kt
        │   │   │   └── Auth.kt
        │   │   └── ui/
        │   │       ├── theme/
        │   │       │   ├── Color.kt
        │   │       │   ├── Type.kt
        │   │       │   └── Theme.kt
        │   │       ├── components/
        │   │       │   └── BottomBar.kt
        │   │       └── screens/
        │   │           ├── SetupScreen.kt
        │   │           ├── LogScreen.kt
        │   │           ├── DashboardScreen.kt
        │   │           ├── GrantsScreen.kt
        │   │           └── SettingsScreen.kt
        │   └── uniffi/                           ← generated, gitignored
        │       └── ohd_storage/
        │           └── ohd_storage.kt            ← from Stage 2
        ├── jniLibs/                              ← generated, gitignored
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
| Android Gradle Plugin | 8.6.1 | `build.gradle.kts` (root) |
| Kotlin | 2.0.21 | `build.gradle.kts` (root) |
| Compose BOM | 2024.10.01 | `app/build.gradle.kts` |
| JNA | 5.14.0 | `app/build.gradle.kts` (uniffi runtime needs ≥5.13) |
| uniffi (Rust) | 0.28 | `storage/crates/ohd-storage-bindings/Cargo.toml` |
| NDK | r26+ (26.1+) | environment, see Prerequisites |
| min SDK | 29 (Android 10) | `app/build.gradle.kts` |
| target SDK | 34 (Android 14) | `app/build.gradle.kts` |
| compile SDK | 34 | `app/build.gradle.kts` |
| Java | 17 | `compileOptions` |

JNA must match the Kotlin codegen target. uniffi 0.28 emits Kotlin that
calls `com.sun.jna.*`; JNA 5.14 ships an Android-flavoured AAR
(`net.java.dev.jna:jna:5.14.0@aar`). The `@aar` suffix matters — the
plain JAR doesn't include the per-ABI native loader stubs Android needs.

## CI considerations

A future GitHub Actions workflow runs Stage 1 + Stage 2 + Stage 3 on the
`ubuntu-latest` runner. Until then, the `.so` files and the generated
`uniffi/ohd_storage.kt` are **not committed** — `.gitignore` excludes
both `app/src/main/jniLibs/` and `app/src/main/java/uniffi/`. Each
contributor regenerates locally.

The Gradle helper task `tasks.register("buildRustCore")` in
`app/build.gradle.kts` documents the cargo-ndk command but **does not
execute it** in this scaffolding phase — running it from Gradle requires
`exec { … }` with the right working directory and propagated environment
variables, which we want to land alongside CI rather than make every
contributor's first `assembleDebug` block on a 3-minute SQLCipher build.

## Troubleshooting

### `cargo ndk` errors out with "linker not found"

`$ANDROID_NDK_HOME` isn't set or points at a stale directory. Verify:

```bash
ls "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt"
# Should print: linux-x86_64 / darwin-x86_64 / darwin-arm64 / windows-x86_64
```

If the directory exists but the linker is still missing, your NDK is
older than r23 — upgrade. cargo-ndk needs the unified `llvm` toolchain.

### `assembleDebug` complains about minSdk / Health Connect

Health Connect requires API 28+ to call but doesn't ship in `play-services-*`
on API 28; it lives in a side-loaded APK on Android 13 and below. The
Connect app declares `minSdk = 29` (Android 10) — older targets are out of
scope. If you're on a Pixel running Android 14+, Health Connect is
preinstalled and the side-load step is unnecessary.

### `UnsatisfiedLinkError` at runtime

```
java.lang.UnsatisfiedLinkError: dlopen failed:
  library "libohd_storage_bindings.so" not found
```

The cdylib for your test device's ABI didn't ship in the APK. Run:

```bash
adb shell getprop ro.product.cpu.abi    # → arm64-v8a (typical)
unzip -l app-debug.apk | grep libohd     # → should list lib/<abi>/libohd_storage_bindings.so
```

If the apk has no `lib/<abi>/` entry for your device's ABI, re-run Stage 1
with that ABI included via `-t`.

### `java.lang.NoClassDefFoundError: com.sun.jna.Native`

JNA isn't on the classpath. Confirm `app/build.gradle.kts` has:

```kotlin
implementation("net.java.dev.jna:jna:5.14.0@aar")
```

The `@aar` suffix is critical — the JVM-only JAR is the default and is
missing the Android `.so` stubs.

### `ProGuard / R8` strips uniffi callbacks in release builds

If a release-mode APK throws `NoSuchMethodError` from inside uniffi
generated code, you need keep rules. Add to `app/proguard-rules.pro`:

```
# Keep uniffi-generated callback interfaces.
-keep class uniffi.** { *; }
-keep class com.sun.jna.** { *; }
```

This is precautionary — uniffi 0.28's Kotlin codegen marks the relevant
classes with annotations R8 already respects.

### "I can't find the AAR"

The Connect Android module currently builds an APK / AAB, not an AAR.
Republishing the OHD core as an `.aar` for downstream Android consumers
(third-party Connect-alikes, reference integrations) is a v1.x deliverable;
see `connect/STATUS.md` "Android".

## Cross-references

- uniffi crate: [`../../storage/crates/ohd-storage-bindings/`](../../storage/crates/ohd-storage-bindings/)
- uniffi surface (Rust): [`../../storage/crates/ohd-storage-bindings/src/lib.rs`](../../storage/crates/ohd-storage-bindings/src/lib.rs)
- iOS recipe (parallel doc): `../ios/BUILD.md` (TBD)
- App architecture rationale: [`../SPEC.md`](../SPEC.md) "Form factors → Android"
- Visual design: [`../../ux-design.md`](../../ux-design.md)

## Self-session OIDC (AppAuth-Android) — wired 2026-05-09

The remote-storage sign-in path lives in
[`app/src/main/java/com/ohd/connect/data/OidcManager.kt`](app/src/main/java/com/ohd/connect/data/OidcManager.kt)
and is invoked from
[`SetupScreen`](app/src/main/java/com/ohd/connect/ui/screens/SetupScreen.kt)
via `rememberLauncherForActivityResult`. Independent of Stages 1–3
above — exercising it doesn't require the Rust core / uniffi codegen
(though the rest of the app does).

### Library

```kotlin
implementation("net.openid:appauth:0.11.1")
implementation("androidx.security:security-crypto:1.1.0-alpha06")
```

`AppAuth-Android` ships its own `RedirectUriReceiverActivity` and
declares it in its library manifest. AGP merges it based on the
`appAuthRedirectScheme` placeholder in
`defaultConfig.manifestPlaceholders`. Set to `com.ohd.connect`.

### Configuration

| Property | Default | Purpose |
|---|---|---|
| `ohd.connect.oidc.storage_url` | (empty — user pastes on first run) | Storage-AS URL the SPA hits for `/.well-known/oauth-authorization-server` (or `/openid-configuration` fallback). |
| `ohd.connect.oidc.client_id` | `ohd-connect-android` | OAuth client_id registered with the storage AS. |
| `ohd.connect.oidc.redirect` | `com.ohd.connect:/oidc-callback` | Custom-scheme redirect URI. **Must** match the registered redirect at the storage AS and the `appAuthRedirectScheme` placeholder. |

Override at build time:

```bash
./gradlew :app:assembleDebug \
  -Pohd.connect.oidc.storage_url=https://ohd.cloud.example \
  -Pohd.connect.oidc.client_id=ohd-connect-android \
  -Pohd.connect.oidc.redirect=com.ohd.connect:/oidc-callback
```

### Token storage

Tokens land in `EncryptedSharedPreferences` (`androidx.security:security-crypto`),
backed by a Keystore-bound AES-256-GCM master key. The
[`Auth`](app/src/main/java/com/ohd/connect/data/Auth.kt) singleton is the
only call site; Keystore-failure → graceful degradation to plain
SharedPreferences with a logged warning. The contract on
`Auth.getSelfSessionToken(...)` etc. is unchanged.

### Flow at runtime

This is **self-session OIDC** — the user signs in against *their own*
storage instance, not against an operator IdP (that's the
`emergency/tablet` story).

1. User picks "Connect to a remote storage" on the Setup screen and
   pastes their storage URL (or the BuildConfig default loads it).
2. `OidcManager.startAuthFlow` calls
   `AuthorizationServiceConfiguration.fetchFromIssuer(storageUrl)` to
   discover the AS metadata; AppAuth tries
   `/.well-known/openid-configuration` first then falls back to
   `/oauth-authorization-server`.
3. AppAuth builds an `AuthorizationRequest` with PKCE and launches the
   storage AS in a Custom Tab.
4. Storage authenticates the user (delegating to whichever upstream
   provider is configured — Google / Apple / Microsoft / OHD account /
   custom OIDC) and redirects to
   `com.ohd.connect:/oidc-callback?code=…&state=…`.
5. `OidcManager.handleAuthResult` exchanges the code for tokens
   (PKCE verifier sent alongside) and persists the `ohds_…` access token
   + refresh + AppAuth state JSON to `Auth`.
6. SetupScreen routes to the Main surface.

### Known gaps

- **Silent refresh wiring** — AppAuth's
  `AuthState.performActionWithFreshTokens` isn't yet threaded into the
  outgoing OHDC HTTP client. The persisted state JSON is ready
  (`Auth.appAuthStateJson(ctx)`); pickup is to honour `expiresAt - 60s`
  in the OHDC interceptor. Mirrors emergency/tablet.
- **RP-initiated logout** — the storage AS's `end_session_endpoint`
  isn't called on sign-out. Local clear via
  `Auth.clearSelfSessionToken(ctx)` already exists.
- **AS metadata discovery via OAuth metadata path** — AppAuth defaults
  to OIDC discovery. Storage v0.x ships only `/openid-configuration`,
  so this is fine for now.
