# OHD Connect — iOS

Swift + SwiftUI. Same OHD Storage Rust core via uniffi (Swift bindings) for
on-device deployments; HTTP/3 (URLSession) for remote primary deployments.

## Status

Scaffold only. `Sources/OhdConnect/ContentView.swift` shows a "OHD Connect v0"
SwiftUI view. The current `Package.swift` declares a SwiftPM library target —
it is **not** a buildable iOS app bundle on its own.

A real iOS deliverable needs an Xcode project + asset catalog + Info.plist,
which the implementation phase will add (likely via Tuist / XcodeGen, or a
checked-in `OhdConnect.xcodeproj/`). The SwiftPM package stays as the entry
for shared modules + tests + macOS-host build.

See [`../STATUS.md`](../STATUS.md) for the full blocker list.

## Requirements

- macOS with Xcode 16 or later
- iOS 17 deployment target (drives SwiftUI feature set, async/await, observation)
- Swift 6 toolchain

## Build / run

```bash
# SwiftPM sanity check (works on macOS or Linux):
swift build

# Open in Xcode:
open Package.swift

# Once an xcodeproj/Tuist setup is in tree (implementation phase):
xcodebuild -project OhdConnect.xcodeproj \
           -scheme OhdConnect \
           -destination 'platform=iOS Simulator,name=iPhone 15' \
           build
```

**Smoke test (not run during scaffolding):** `swift build` in this directory.
Expected to succeed once the OHDC Swift client lands at
`../shared/ohdc-clients/swift/`.

## Layout

```
ios/
├── Package.swift              # SwiftPM manifest
└── Sources/OhdConnect/
    └── ContentView.swift      # SwiftUI v0
```

## OHDC client

The remote-primary path will use a hand-rolled URLSession + Connect-Protocol JSON client mirroring the Android approach (see [`../android/BUILD.md`](../android/BUILD.md) "OHDC client"). Currently absent.

## On-device storage

For the on-device deployment mode, the app links the OHD Storage Rust core via uniffi's Swift bindings from [`../../storage/crates/ohd-storage-bindings/`](../../storage/crates/ohd-storage-bindings/). The xcframework recipe is sketched in that crate's README; iOS-specific BUILD.md lands alongside the Xcode project.

## HealthKit bridge

Parallel to the Android Health Connect bridge. Holds an `ohdd_…` device token
and pulls from HealthKit on a background-task schedule. Specific permission
list, query strategies, and bridge spec are TBD (the Connect-side
`spec/health-connect.md` is Android-specific).

## APNs critical-alert

For the emergency dialog, the app needs the `com.apple.developer.usernotifications.critical-alerts`
entitlement and a registered critical-alert sound. Apple requires explicit
user opt-in (OS prompt) before critical alerts fire. The entitlement
itself requires App Store review approval.

Per [`../spec/notifications.md`](../spec/notifications.md) and
[`../spec/screens-emergency.md`](../spec/screens-emergency.md).

## License

Dual-licensed `Apache-2.0 OR MIT`, matching the project root.
