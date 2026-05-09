# R8 / ProGuard keep rules for OHD Emergency.
#
# uniffi 0.28's Kotlin codegen marks the relevant classes with @Keep
# annotations R8 already respects. These rules are precautionary:
# the Stage 1/2 cdylib + JNA bridge use reflection at runtime to
# resolve native function names; R8 stripping those symbols would
# crash the app at first FFI call.

# Keep uniffi-generated callback interfaces and the JNA runtime.
-keep class uniffi.** { *; }
-keep class com.sun.jna.** { *; }

# Compose / Kotlin metadata is kept by AGP defaults; nothing else
# Compose-specific to add here.

# OkHttp 4 — published rules at https://square.github.io/okhttp/r8_proguard/.
# OkHttp internally relies on platform / environment detection that R8
# can otherwise eliminate; the dontwarn entries below are harmless on
# Android (the corresponding TLS providers don't ship on the platform).
-dontwarn okhttp3.internal.platform.**
-dontwarn org.conscrypt.**
-dontwarn org.bouncycastle.**
-dontwarn org.openjsse.**
# Okio (transitive of OkHttp).
-dontwarn okio.**

# OHDC client — the DTOs use reflective JSON access (`org.json`); R8
# field-stripping is safe but we keep them to make stack traces from a
# JSON-shape mismatch readable in the wild. When Moshi /
# kotlinx-serialization land alongside binary-protobuf codegen, revisit
# per their codegen requirements.
-keep class com.ohd.emergency.data.ohdc.** { *; }
