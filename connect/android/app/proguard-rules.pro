# ProGuard / R8 rules for OHD Connect Android.
#
# The Compose plugin already keeps Compose runtime metadata; this file is
# the project-specific layer.

# uniffi-generated Kotlin façade lives at `package uniffi.ohd_storage`.
# The codegen marks the relevant classes with annotations R8 already
# respects, but we keep the entire package to be safe — release-build
# regressions here surface as `NoSuchMethodError` from inside generated
# code, with an unhelpful stack trace.
-keep class uniffi.** { *; }

# JNA's runtime reflection requires class + method preservation for every
# Structure / Callback / Library subclass we hand it. uniffi's Kotlin
# codegen subclasses `com.sun.jna.Structure` for record types and
# `com.sun.jna.Callback` for closures; keep the whole namespace.
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.Structure { *; }
-keepclassmembers class * extends com.sun.jna.Callback { *; }

# Compose runtime keeps function references via reflection in some code
# paths; the Compose plugin's default rules cover this but adding an
# explicit kotlinx.coroutines keep avoids debug-info-only issues.
-keep class kotlinx.coroutines.** { *; }

# OHDC client (when it lands) may need keep rules for Protobuf-generated
# message types. Add them here once codegen is wired.
