package com.ohd.connect.data

/**
 * Static OSS attribution registry for the Connect Android build.
 *
 * Each entry corresponds to a third-party dependency declared in
 * `app/build.gradle.kts` (or the Compose BOM that resolves a family of
 * Compose modules to a single coordinated version). Versions are tracked
 * manually for v1 — when the dependency block changes, update the matching
 * `OssLib` line below. A future iteration can read this from a
 * Gradle-emitted manifest, but for the beta the source of truth is hand-
 * curated to keep the list short, ordered, and human-readable.
 *
 * Used by:
 *  - `ui/screens/settings/LicencesScreen.kt` — in-app list under
 *    Settings → About → Open-source licences.
 *  - The marketing site mirrors the same shape at `landing/credits.html`;
 *    the two are kept in lock-step by convention (not automatically).
 *
 * Don't fabricate licences — if a dep ships under a custom-licence terms
 * page (Google Play Services), surface the upstream link and mark it with
 * the dedicated [Licence.GOOGLE_PLAY_SERVICES] enum.
 */
object OssLicences {
    /** Logical grouping for the list UI. */
    enum class Category(val display: String) {
        SelfLicence("OHD Connect"),
        Jetpack("Jetpack & AndroidX"),
        PlayServices("Google Play Services"),
        Other("Other libraries"),
        Tooling("Tooling & tests"),
        Native("Native dependencies (Rust core)"),
        Data("Data sources"),
    }

    /**
     * SPDX-ish enum.
     *
     * `display` is the human-readable name for the listing row. `spdx` is
     * the machine-readable SPDX identifier (or a `LicenseRef-…` placeholder
     * for non-SPDX terms like Google Play Services). The Apache-2.0 entry
     * doubles for dual-licensed projects (e.g. JNA Apache-2.0 OR LGPL — we
     * elect the Apache option, which is permitted).
     */
    enum class Licence(val display: String, val spdx: String) {
        APACHE_2_0("Apache License 2.0", "Apache-2.0"),
        MIT("MIT License", "MIT"),
        BSD_3_CLAUSE("BSD 3-Clause", "BSD-3-Clause"),
        UNLICENSE("Unlicense", "Unlicense"),
        EPL_1_0("Eclipse Public License 1.0", "EPL-1.0"),
        ODBL_1_0("Open Database License 1.0", "ODbL-1.0"),
        APACHE_2_0_OR_MIT("Apache-2.0 OR MIT", "Apache-2.0 OR MIT"),
        GOOGLE_PLAY_SERVICES(
            "Google Play Services Terms",
            "LicenseRef-GooglePlayServices",
        ),
    }

    data class OssLib(
        val name: String,
        val groupArtifact: String,
        val version: String,
        val licence: Licence,
        val url: String,
        val category: Category,
        /** Optional one-line note shown muted under the metadata row. */
        val note: String? = null,
    )

    /**
     * The full attribution list. Ordering inside each category is the
     * order entries should appear in the rendered list.
     *
     * Versions track `app/build.gradle.kts`:
     *  - Compose modules — `compose-bom:2024.10.01` (BOM-managed).
     *  - CameraX — `1.4.1` (`cameraxVersion`).
     *  - Everything else — pinned directly in the dependency block.
     */
    val all: List<OssLib> = listOf(
        // ----- Self -----
        OssLib(
            name = "OHD Connect (this app)",
            groupArtifact = "com.ohd.connect",
            version = com.ohd.connect.BuildConfig.VERSION_NAME,
            licence = Licence.APACHE_2_0_OR_MIT,
            url = "https://github.com/ohd-foundation/ohd",
            category = Category.SelfLicence,
            note = "Dual-licensed — pick whichever fits your project.",
        ),

        // ----- Jetpack & AndroidX -----
        OssLib(
            name = "Jetpack Compose (UI)",
            groupArtifact = "androidx.compose.ui:ui",
            version = "BOM 2024.10.01",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/compose-ui",
            category = Category.Jetpack,
        ),
        OssLib(
            name = "Jetpack Compose (Foundation)",
            groupArtifact = "androidx.compose.foundation:foundation",
            version = "BOM 2024.10.01",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/compose-foundation",
            category = Category.Jetpack,
        ),
        OssLib(
            name = "Jetpack Compose Material 3",
            groupArtifact = "androidx.compose.material3:material3",
            version = "BOM 2024.10.01",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/compose-material3",
            category = Category.Jetpack,
        ),
        OssLib(
            name = "Material Icons Extended",
            groupArtifact = "androidx.compose.material:material-icons-extended",
            version = "BOM 2024.10.01",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/compose-material",
            category = Category.Jetpack,
        ),
        OssLib(
            name = "Navigation Compose",
            groupArtifact = "androidx.navigation:navigation-compose",
            version = "2.8.4",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/navigation",
            category = Category.Jetpack,
        ),
        OssLib(
            name = "Lifecycle Runtime KTX",
            groupArtifact = "androidx.lifecycle:lifecycle-runtime-ktx",
            version = "2.8.7",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/lifecycle",
            category = Category.Jetpack,
        ),
        OssLib(
            name = "Activity Compose",
            groupArtifact = "androidx.activity:activity-compose",
            version = "1.9.3",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/activity",
            category = Category.Jetpack,
        ),
        OssLib(
            name = "Core KTX",
            groupArtifact = "androidx.core:core-ktx",
            version = "1.13.1",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/core",
            category = Category.Jetpack,
        ),
        OssLib(
            name = "Security Crypto",
            groupArtifact = "androidx.security:security-crypto",
            version = "1.1.0-alpha06",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/security",
            category = Category.Jetpack,
            note = "EncryptedSharedPreferences for the self-session bearer + AppAuth state.",
        ),
        OssLib(
            name = "Health Connect Client",
            groupArtifact = "androidx.health.connect:connect-client",
            version = "1.1.0-alpha07",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/health-connect",
            category = Category.Jetpack,
        ),
        OssLib(
            name = "CameraX Core",
            groupArtifact = "androidx.camera:camera-core",
            version = "1.4.1",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/camera",
            category = Category.Jetpack,
        ),
        OssLib(
            name = "CameraX Camera2 / Lifecycle / View",
            groupArtifact = "androidx.camera:camera-{camera2,lifecycle,view}",
            version = "1.4.1",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/camera",
            category = Category.Jetpack,
        ),
        OssLib(
            name = "WorkManager",
            groupArtifact = "androidx.work:work-runtime-ktx",
            version = "2.9.1",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/work",
            category = Category.Jetpack,
        ),

        // ----- Google Play Services -----
        OssLib(
            name = "Google Play Services — Code Scanner",
            groupArtifact = "com.google.android.gms:play-services-code-scanner",
            version = "16.1.0",
            licence = Licence.GOOGLE_PLAY_SERVICES,
            url = "https://developers.google.com/ml-kit/code-scanner",
            category = Category.PlayServices,
        ),
        OssLib(
            name = "ML Kit Barcode Scanning (unbundled)",
            groupArtifact = "com.google.android.gms:play-services-mlkit-barcode-scanning",
            version = "18.3.1",
            licence = Licence.GOOGLE_PLAY_SERVICES,
            url = "https://developers.google.com/ml-kit/vision/barcode-scanning",
            category = Category.PlayServices,
        ),

        // ----- Other libraries -----
        OssLib(
            name = "AppAuth-Android",
            groupArtifact = "net.openid:appauth",
            version = "0.11.1",
            licence = Licence.APACHE_2_0,
            url = "https://github.com/openid/AppAuth-Android",
            category = Category.Other,
            note = "OIDC code-flow + PKCE against your storage authorisation server.",
        ),
        OssLib(
            name = "JNA (Java Native Access)",
            groupArtifact = "net.java.dev.jna:jna",
            version = "5.14.0",
            licence = Licence.APACHE_2_0,
            url = "https://github.com/java-native-access/jna",
            category = Category.Other,
            note = "Dual-licensed Apache-2.0 OR LGPL-2.1+; OHD elects Apache-2.0.",
        ),
        OssLib(
            name = "Guava (Android)",
            groupArtifact = "com.google.guava:guava",
            version = "33.3.1-android",
            licence = Licence.APACHE_2_0,
            url = "https://github.com/google/guava",
            category = Category.Other,
        ),
        OssLib(
            name = "Kotlin Standard Library",
            groupArtifact = "org.jetbrains.kotlin:kotlin-stdlib",
            version = "1.9+",
            licence = Licence.APACHE_2_0,
            url = "https://kotlinlang.org/",
            category = Category.Other,
        ),
        OssLib(
            name = "Kotlin Coroutines",
            groupArtifact = "org.jetbrains.kotlinx:kotlinx-coroutines-core",
            version = "1.8+",
            licence = Licence.APACHE_2_0,
            url = "https://github.com/Kotlin/kotlinx.coroutines",
            category = Category.Other,
        ),

        // ----- Tooling & tests -----
        OssLib(
            name = "JUnit 4",
            groupArtifact = "junit:junit",
            version = "4.13.2",
            licence = Licence.EPL_1_0,
            url = "https://junit.org/junit4/",
            category = Category.Tooling,
        ),
        OssLib(
            name = "AndroidX Test — ext:junit",
            groupArtifact = "androidx.test.ext:junit",
            version = "1.2.1",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/test",
            category = Category.Tooling,
        ),
        OssLib(
            name = "AndroidX Test — rules",
            groupArtifact = "androidx.test:rules",
            version = "1.6.1",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/jetpack/androidx/releases/test",
            category = Category.Tooling,
        ),
        OssLib(
            name = "Espresso Core",
            groupArtifact = "androidx.test.espresso:espresso-core",
            version = "3.6.1",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/training/testing/espresso",
            category = Category.Tooling,
        ),
        OssLib(
            name = "UI Automator",
            groupArtifact = "androidx.test.uiautomator:uiautomator",
            version = "2.3.0",
            licence = Licence.APACHE_2_0,
            url = "https://developer.android.com/training/testing/other-components/ui-automator",
            category = Category.Tooling,
        ),
        OssLib(
            name = "Paparazzi (screenshot tests)",
            groupArtifact = "app.cash.paparazzi:paparazzi",
            version = "Gradle plugin",
            licence = Licence.APACHE_2_0,
            url = "https://github.com/cashapp/paparazzi",
            category = Category.Tooling,
        ),

        // ----- Native dependencies (Rust core) -----
        OssLib(
            name = "OHD Storage core + transitive Rust crates",
            groupArtifact = "see storage/Cargo.toml",
            version = "see upstream",
            licence = Licence.APACHE_2_0_OR_MIT,
            url = "https://github.com/ohd-foundation/ohd/tree/main/storage",
            category = Category.Native,
            note = "Includes rusqlite, uniffi, connectrpc, quinn, openssl-src, and others bundled inside the `.so` per ABI.",
        ),

        // ----- Data sources -----
        OssLib(
            name = "Open Food Facts (product database)",
            groupArtifact = "data.openfoodfacts.org",
            version = "rolling",
            licence = Licence.ODBL_1_0,
            url = "https://openfoodfacts.org",
            category = Category.Data,
            note = "Open Database Licence — covers product data only, not code.",
        ),
    )

    /** Library count for the Settings hub summary row. */
    val count: Int get() = all.size

    /** Entries grouped by [Category], preserving the order in [all]. */
    val byCategory: List<Pair<Category, List<OssLib>>>
        get() = all.groupBy { it.category }.entries.map { it.key to it.value }
}
