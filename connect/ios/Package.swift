// swift-tools-version:5.9
import PackageDescription

// SwiftPM manifest for the OHD Connect iOS app.
//
// In v0 this is a library target so `swift build` works on any host. The
// implementation phase will add an Xcode project (or Tuist setup) that
// declares the actual iOS app bundle target, asset catalog, Info.plist,
// entitlements (HealthKit, critical alerts, notifications, BLE), etc.
//
// Why we keep Package.swift around once the xcodeproj exists: SwiftPM is
// still the cleanest way to vendor the OHDC Swift client, share view-model
// modules between iOS and (potentially) macOS, and run unit tests on a Linux
// CI host.

let package = Package(
    name: "OhdConnect",
    platforms: [
        .iOS(.v17),
        .macOS(.v14),
    ],
    products: [
        .library(
            name: "OhdConnect",
            targets: ["OhdConnect"]
        ),
    ],
    dependencies: [
        // The OHDC Swift client lands at ../shared/ohdc-clients/swift once
        // the storage component publishes its first codegen drop. Wire it as:
        //   .package(path: "../shared/ohdc-clients/swift")
        // or via a SwiftPM remote URL (TBD).
    ],
    targets: [
        .target(
            name: "OhdConnect",
            dependencies: [
                // .product(name: "OhdcClient", package: "ohdc-client-swift"),
            ],
            path: "Sources/OhdConnect"
        ),
    ]
)
