// Top-level build file for the OHD Emergency paramedic tablet app.
//
// Module-level config lives in `app/build.gradle.kts`; the catalogue of
// pinned versions lives in `gradle/libs.versions.toml`. This file just
// declares the plugins the :app module applies — keeping the version
// pins out of `apply false` clauses by routing them through the catalogue.
//
// See `BUILD.md` for the three-stage build recipe (cargo-ndk → uniffi-bindgen
// → ./gradlew assembleRelease). The Rust core path is a thin wrapper around
// the same `ohd-storage-bindings` crate that connect/android consumes; the
// emergency app uses uniffi only for the local case-vault cache (active-case
// snapshot + queued offline writes), with the OHDC HTTP client doing the
// real work against the operator's relay.

plugins {
    alias(libs.plugins.android.application) apply false
    alias(libs.plugins.kotlin.android) apply false
    alias(libs.plugins.kotlin.compose) apply false
}
