# OHD Connect — Android

Kotlin + Jetpack Compose. Links the OHD Storage Rust core via uniffi for
on-device deployments; HTTP/3 (Cronet) for remote primary deployments.

## Status

Compose UI with a working Setup → Main flow. Four-tab bottom-bar shell
(Log / Dashboard / Grants / Settings) wired against `data/StorageRepository.kt`,
which wraps the uniffi handle from `../../storage/crates/ohd-storage-bindings/`.
Self-session OIDC (AppAuth-Android) is wired end-to-end against the storage
AS — see [`BUILD.md`](BUILD.md) "Self-session OIDC".

The bindings crate compiles cleanly; the `.so` files + Kotlin façade are
generated locally per [`BUILD.md`](BUILD.md) (NDK + cargo-ndk + uniffi-bindgen)
on first checkout — they're gitignored.

See [`../STATUS.md`](../STATUS.md) for the full Connect-component status
and [`BUILD.md`](BUILD.md) for the developer build recipe.

## Requirements

- Android Studio Hedgehog (2023.1.1) or later
- JDK 17
- Android SDK with API 34 (target) and API 29 (min)
- Android NDK r26+ (for the Rust core cross-build; see [`BUILD.md`](BUILD.md))
- `cargo-ndk` 3.5+ (for the Rust core cross-build)
- Health Connect APK installed on the test device (Android 13 and earlier)

## Build / run

The full three-stage recipe — Rust core → Kotlin bindings → Gradle assemble — lives in [`BUILD.md`](BUILD.md). After the first cargo-ndk + uniffi-bindgen pass:

```bash
# From this directory:
./gradlew :app:assembleDebug              # produce app/build/outputs/apk/debug/app-debug.apk
./gradlew :app:installDebug               # install on connected device
./gradlew :app:lint                       # lint pass

# Or open in Android Studio:
#   File → Open → connect/android
```

If the Gradle wrapper isn't checked in, run `gradle wrapper` once (or let Android Studio bootstrap it on import).

## Layout

```
android/
├── BUILD.md                    # Stage 1 + Stage 2 + Stage 3 build recipe
├── settings.gradle.kts         # root settings, includes :app
├── build.gradle.kts            # root build script (plugin versions)
├── gradle.properties           # JVM args, AndroidX flags
└── app/
    ├── build.gradle.kts        # Compose + JNA + buildRustCore task
    ├── proguard-rules.pro      # uniffi / JNA keep rules
    └── src/main/
        ├── AndroidManifest.xml
        ├── res/                # strings.xml + data_extraction_rules.xml
        └── java/
            ├── com/ohd/connect/
            │   ├── MainActivity.kt
            │   ├── data/
            │   │   ├── StorageRepository.kt  # wraps the uniffi handle
            │   │   └── Auth.kt               # token + first-run state
            │   └── ui/
            │       ├── theme/                # Material3, dark default
            │       ├── components/
            │       │   └── BottomBar.kt
            │       └── screens/
            │           ├── SetupScreen.kt
            │           ├── LogScreen.kt
            │           ├── DashboardScreen.kt
            │           ├── GrantsScreen.kt
            │           └── SettingsScreen.kt
            └── uniffi/                       # generated, gitignored
                └── ohd_storage/ohd_storage.kt
```

## OHDC client

The remote-deployment path uses a hand-rolled OkHttp + Connect-Protocol JSON
client — see [`BUILD.md`](BUILD.md) "OHDC client". The on-device path uses
uniffi to talk to the Rust core in-process and skips the wire entirely.

A Buf-generated Kotlin client may replace the hand-rolled one once the
storage component publishes its first Kotlin codegen drop; the
`EmergencyRepository`-style facade in `data/StorageRepository.kt` shields
the rest of the app from that swap.

## On-device storage

For the on-device deployment mode (default for new Android users), the app
links the OHD Storage Rust core via uniffi. The Rust → Kotlin binding ships
out of `../../storage/crates/ohd-storage-bindings/` (storage component owns
this). [`BUILD.md`](BUILD.md) documents the cross-compile + bindgen flow.

## Health Connect bridge

A separate Foreground Service holds an `ohdd_…` device token and pulls from
Health Connect on a WorkManager schedule. Per
[`../spec/health-connect.md`](../spec/health-connect.md). Permissions list
and architecture sketch in that doc. Not wired in v0.

## Emergency dialog above lock screen

Implemented via:

- `Notification.Builder.setFullScreenIntent(...)` with a high-priority FCM
  channel.
- An Activity flagged `FLAG_SHOW_WHEN_LOCKED | FLAG_DISMISS_KEYGUARD` for the
  dialog itself.
- Vibration + alert sound continuous-loop until user acts or timeout.

Per [`../spec/screens-emergency.md`](../spec/screens-emergency.md). Not
wired in v0.

## Background sync constraints

WorkManager periodic with min interval 15 minutes. Restrict to idle + not-low-
battery per Health Connect best practice.

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root.
