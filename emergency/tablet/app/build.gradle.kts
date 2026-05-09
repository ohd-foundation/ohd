plugins {
    alias(libs.plugins.android.application)
    alias(libs.plugins.kotlin.android)
    alias(libs.plugins.kotlin.compose)
}

android {
    namespace = "com.ohd.emergency"
    compileSdk = 35

    defaultConfig {
        applicationId = "com.ohd.emergency"
        // minSdk 30: BLUETOOTH_SCAN with `neverForLocation` was introduced in
        // API 31, but we keep the floor at 30 so legacy 7-year-old rugged
        // tablets (still common in EU EMS fleets) can install. Pre-31
        // devices use the legacy BLUETOOTH + ACCESS_FINE_LOCATION path
        // (declared in AndroidManifest.xml with `maxSdkVersion=30`).
        minSdk = 30
        targetSdk = 35
        versionCode = 1
        versionName = "0.0.1"

        // The cdylibs Stage 1 (cargo-ndk) drops into `app/src/main/jniLibs/<abi>/`
        // cover these. Keep this list in sync with the `-t` flags in BUILD.md.
        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64")
        }

        // AppAuth-Android consumes the OAuth redirect via a manifest
        // placeholder. The custom-scheme redirect MUST match the value
        // baked into BuildConfig below + the IdP's registered redirect URI.
        // Format: <scheme>:/<path> — no host, custom scheme. The placeholder
        // populates the AndroidManifest's RedirectUriReceiverActivity intent
        // filter automatically (AppAuth's library manifest declares it).
        manifestPlaceholders["appAuthRedirectScheme"] = "com.ohd.emergency"

        // BuildConfig fields populate the LoginScreen defaults. Override per
        // deployment by editing this file or passing -P flags from CI.
        buildConfigField(
            "String",
            "OHD_EMERGENCY_OIDC_ISSUER",
            "\"${project.findProperty("ohd.emergency.oidc.issuer") ?: "https://idp.example.cz/realms/ems"}\"",
        )
        buildConfigField(
            "String",
            "OHD_EMERGENCY_OIDC_CLIENT_ID",
            "\"${project.findProperty("ohd.emergency.oidc.client_id") ?: "ohd-emergency-tablet"}\"",
        )
        buildConfigField(
            "String",
            "OHD_EMERGENCY_OIDC_REDIRECT",
            "\"${project.findProperty("ohd.emergency.oidc.redirect") ?: "com.ohd.emergency:/oidc-callback"}\"",
        )

        // Operator's relay base URL. Override via:
        //   ./gradlew :app:assembleDebug -Pohd.emergency.relay.base=https://relay.ems-prague.cz
        // Falls through to dev-loopback (10.0.2.2 = host on Android
        // emulator) so a demo against `cargo run -- serve --port 8443`
        // on the dev machine works out of the box.
        buildConfigField(
            "String",
            "OHD_EMERGENCY_RELAY_BASE",
            "\"${project.findProperty("ohd.emergency.relay.base") ?: "http://10.0.2.2:8443"}\"",
        )
    }

    buildTypes {
        getByName("debug") {
            isMinifyEnabled = false
            isJniDebuggable = true
        }
        getByName("release") {
            isMinifyEnabled = true
            proguardFiles(
                getDefaultProguardFile("proguard-android-optimize.txt"),
                "proguard-rules.pro"
            )
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }

    kotlinOptions {
        jvmTarget = "17"
    }

    buildFeatures {
        compose = true
        buildConfig = true
    }

    // Keep per-ABI `.so` files un-recompressed inside the APK so dlopen()
    // can mmap them in place. AGP-recommended whenever jniLibs ship.
    packaging {
        jniLibs {
            useLegacyPackaging = false
        }
    }

    // Source sets — generated Kotlin from uniffi-bindgen lands at
    // `app/src/main/java/uniffi/`. AGP includes `src/main/java/` by default
    // but listing it explicitly keeps the layout obvious.
    sourceSets {
        getByName("main") {
            java.srcDirs("src/main/java")
            jniLibs.srcDirs("src/main/jniLibs")
        }
    }
}

dependencies {
    // Compose BOM — every Compose dep tracks the BOM version.
    val composeBom = platform(libs.androidx.compose.bom)
    implementation(composeBom)
    androidTestImplementation(composeBom)

    implementation(libs.androidx.core.ktx)
    implementation(libs.androidx.lifecycle.runtime.ktx)
    implementation(libs.androidx.lifecycle.viewmodel.compose)
    implementation(libs.androidx.activity.compose)
    implementation(libs.androidx.navigation.compose)

    implementation(libs.androidx.compose.ui)
    implementation(libs.androidx.compose.ui.graphics)
    implementation(libs.androidx.compose.ui.tooling.preview)
    implementation(libs.androidx.compose.material3)
    implementation(libs.androidx.compose.material.icons.core)
    implementation(libs.androidx.compose.material.icons.extended)

    implementation(libs.kotlinx.coroutines.core)
    implementation(libs.kotlinx.coroutines.android)

    // JNA (Java Native Access) — uniffi 0.28's Kotlin codegen routes every
    // FFI call through `com.sun.jna.Native.register(...)`. The `@aar`
    // suffix is critical: the plain JAR is missing the per-ABI Android
    // native loader stubs and produces `UnsatisfiedLinkError` at runtime.
    // 5.14+ is the floor; 5.13 doesn't ship a stable Android AAR.
    implementation("${libs.jna.get().module}:${libs.jna.get().versionConstraint.requiredVersion}@aar")

    // Tests: JUnit + coroutines-test for the repository / OhdcClient
    // unit tests. MockWebServer provides a local HTTP/2 server for the
    // OkHttp-side tests of OhdcClient (see test/.../OhdcClientTest.kt).
    testImplementation(libs.junit)
    testImplementation("org.jetbrains.kotlinx:kotlinx-coroutines-test:1.9.0")
    testImplementation("com.squareup.okhttp3:mockwebserver:4.12.0")
    androidTestImplementation(libs.androidx.test.ext.junit)
    androidTestImplementation(libs.androidx.test.espresso)
    androidTestImplementation(libs.androidx.compose.ui.test.junit4)
    debugImplementation(libs.androidx.compose.ui.tooling)
    debugImplementation(libs.androidx.compose.ui.test.manifest)

    // EncryptedSharedPreferences for the operator OIDC bearer. The
    // OperatorSession singleton wraps this; the API surface is identical
    // to plain SharedPreferences. Floor pinned to the last stable 1.1.x
    // — 1.1.0-alpha07 is the API that ships KeyScheme.AES256_GCM (the
    // shape OperatorSession uses).
    implementation("androidx.security:security-crypto:1.1.0-alpha06")

    // AppAuth-Android — operator-OIDC code flow + PKCE. Launches the IdP
    // in a Custom Tab and persists the OAuth state via its own AuthState.
    // Mirrors the connect/web `oauth4webapi` shape.
    implementation("net.openid:appauth:0.11.1")

    // OkHttp — HTTP/2 client for the OHDC Connect-Protocol surface and
    // the relay's emergency endpoints. JSON encoding via stdlib `org.json`
    // (already on Android); no separate JSON library needed for v0.
    // 4.12.x is the current stable line; matches what most Android apps
    // pin and avoids the alpha 5.x line.
    implementation("com.squareup.okhttp3:okhttp:4.12.0")

    // OHDC Kotlin client — hand-rolled OkHttp + Connect-Protocol JSON
    // (lives at `data/ohdc/OhdcClient.kt`). Documented choice over
    // Connect-Kotlin in OhdcClient's KDoc. When the storage component
    // publishes binary-protobuf codegen drops, this surface gains a
    // sibling `OhdcBinaryClient` that wraps the generated stubs.

    // Future deps that land alongside real wiring:
    //  - androidx.work:work-runtime-ktx (offline write queue flush worker)
    //  - androidx.biometric:biometric (panic-logout / shift-in unlock)
    //  - cronet (HTTP/3 transport for OHDC, once available)
    //  - kotlinx-serialization (heavier JSON; org.json suffices for v0)
}

// =============================================================================
// Documentation-only Rust core build task.
//
// Mirrors connect/android. Real runners would hook this into `preBuild`
// or generate the `.so` files during a CI job; for the scaffolding phase
// the task only prints the command developers should run manually. See
// `BUILD.md` for the full recipe.
//
// The emergency tablet differs from connect in WHY it needs uniffi:
//   - connect/android uses uniffi for the primary on-device storage path
//     (mode A: data lives in a SQLCipher file on the device).
//   - emergency/tablet uses uniffi for a *local cache* (active case
//     snapshot + queued offline intervention writes), with the
//     authoritative path being OHDC HTTP/3 to the operator's relay
//     (mode B: data lives at the operator's relay-mediated remote
//     storage, the tablet reflects).
//
// Same Rust crate (`ohd-storage-bindings`), different deployment mode.
// =============================================================================
tasks.register("buildRustCore") {
    group = "ohd"
    description = "Cross-compile the Rust core into per-ABI .so files via cargo-ndk."
    doLast {
        val cmd = """
            cd ../../storage/crates/ohd-storage-bindings
            cargo ndk \
              -t arm64-v8a \
              -t armeabi-v7a \
              -t x86_64 \
              -o ../../../emergency/tablet/app/src/main/jniLibs \
              build --release
        """.trimIndent()
        println("== buildRustCore: run this manually ==")
        println(cmd)
        println("Then regenerate Kotlin bindings:")
        println("    cd storage")
        println("    cargo run --features cli --bin uniffi-bindgen -- \\")
        println("      generate \\")
        println("      --library target/release/libohd_storage_bindings.so \\")
        println("      --language kotlin \\")
        println("      --out-dir ../emergency/tablet/app/src/main/java/uniffi")
        println("== see emergency/tablet/BUILD.md for the full recipe ==")
    }
}
