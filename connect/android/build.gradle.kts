// Top-level build file. Plugin versions live here; module-level
// build.gradle.kts files apply them.

plugins {
    id("com.android.application") version "8.6.1" apply false
    id("org.jetbrains.kotlin.android") version "2.0.21" apply false
    id("org.jetbrains.kotlin.plugin.compose") version "2.0.21" apply false
    // Paparazzi — JVM-based Compose screenshot regression. Pinned to the
    // last release that ships against AGP 8.x + Kotlin 2.0.x. The plugin is
    // applied in `app/build.gradle.kts`; declaring it here lets root-level
    // tasks (CI parallel evaluation) resolve the plugin classpath without
    // every subproject re-fetching.
    id("app.cash.paparazzi") version "1.3.5" apply false
}
