plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
    // Paparazzi screenshot tests — `./gradlew :app:recordPaparazziDebug`
    // captures the baseline under `app/src/test/snapshots/`, then
    // `:app:verifyPaparazziDebug` (run by `:app:test`) is the regression
    // gate. The plugin only runs on the `Debug` variant by default which
    // matches our spec — release-only differences aren't worth screenshot
    // gating.
    id("app.cash.paparazzi")
}

android {
    namespace = "com.ohd.connect"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.ohd.connect"
        minSdk = 29
        targetSdk = 34
        versionCode = 46
        versionName = "0.1.0-beta46"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"

        // The cdylibs Stage 1 (cargo-ndk) drops into
        // `app/src/main/jniLibs/<abi>/` cover these. Keep this list in
        // sync with the `-t` flags in `BUILD.md` Stage 1.
        ndk {
            abiFilters += listOf("arm64-v8a", "armeabi-v7a", "x86_64")
        }

        // AppAuth-Android consumes the OAuth redirect via a manifest
        // placeholder. The custom-scheme redirect MUST match the value
        // baked into BuildConfig below + the storage AS's registered
        // redirect URI. Format: <scheme>:/<path> — no host. AppAuth's
        // library manifest declares a `RedirectUriReceiverActivity` that
        // catches this scheme and bounces back into the app.
        manifestPlaceholders["appAuthRedirectScheme"] = "com.ohd.connect"

        // BuildConfig fields populate the SetupScreen "Connect to a remote
        // storage" defaults. Storage URL is necessarily a placeholder per
        // deployment; the user pastes their own on first run. CI / fleet
        // deployments can pass `-Pohd.connect.oidc.storage_url=...` to
        // pre-fill.
        buildConfigField(
            "String",
            "OHD_OIDC_STORAGE_URL",
            "\"${project.findProperty("ohd.connect.oidc.storage_url") ?: ""}\"",
        )
        buildConfigField(
            "String",
            "OHD_OIDC_CLIENT_ID",
            "\"${project.findProperty("ohd.connect.oidc.client_id") ?: "ohd-connect-android"}\"",
        )
        buildConfigField(
            "String",
            "OHD_OIDC_REDIRECT",
            "\"${project.findProperty("ohd.connect.oidc.redirect") ?: "com.ohd.connect:/oidc-callback"}\"",
        )
    }

    buildTypes {
        debug {
            isMinifyEnabled = false
            isJniDebuggable = true
        }
        release {
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
        // BuildConfig is needed by `SettingsScreen` (BuildConfig.VERSION_NAME).
        buildConfig = true
    }

    // Keep the per-ABI `.so` files un-recompressed inside the APK so dlopen()
    // can mmap them in place. This is the AGP-recommended setting whenever
    // jniLibs ship and matters for cold-start performance.
    packaging {
        jniLibs {
            useLegacyPackaging = false
        }
    }

    // Source sets — the generated Kotlin from uniffi-bindgen lands at
    // `app/src/main/java/uniffi/`. AGP includes `src/main/java/` by default,
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
    val composeBom = platform("androidx.compose:compose-bom:2024.10.01")
    implementation(composeBom)
    androidTestImplementation(composeBom)

    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.lifecycle:lifecycle-runtime-ktx:2.8.7")
    implementation("androidx.activity:activity-compose:1.9.3")
    // Storage Access Framework wrappers — used by the Samsung ECG importer
    // to enumerate CSVs inside a user-picked folder tree.
    implementation("androidx.documentfile:documentfile:1.0.1")

    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-core")
    implementation("androidx.compose.material:material-icons-extended")

    // Navigation-Compose — wires the four-tab bottom bar + nested loggers
    // (HomeScreen → Medication/Food/Symptom/Measurement/UrineStrip/FormBuilder),
    // Settings hub → Access/Storage/etc., and the operator stack reachable
    // from Settings → Profile & Access. See `ui/nav/NavGraph.kt`.
    implementation("androidx.navigation:navigation-compose:2.8.4")

    // JNA (Java Native Access) — uniffi 0.28's Kotlin codegen routes every
    // FFI call through `com.sun.jna.Native.register(...)`. The `@aar`
    // suffix is critical: the plain JAR is missing the per-ABI Android
    // native loader stubs and produces `UnsatisfiedLinkError` at runtime.
    // 5.14+ is the floor; 5.13 doesn't ship a stable Android AAR.
    implementation("net.java.dev.jna:jna:5.14.0@aar")

    // -----------------------------------------------------------------
    // Tests — see `app/src/androidTest/.../SmokeTest.kt` and
    // `app/src/test/.../PencilScreenshotsTest.kt`.
    //
    //   - junit4 + AndroidX `androidx.test.ext:junit` — runner used by
    //     instrumentation. The `test:runner` floor matches AGP 8.6.x.
    //   - `compose.ui:ui-test-junit4` — `createAndroidComposeRule` /
    //     `onNodeWithText`. Pulled by the BOM, version is implicit.
    //   - `compose.ui:ui-test-manifest` (debug only) — adds the
    //     `<activity android:name="ComponentActivity">` declaration that
    //     `createAndroidComposeRule` needs at instrumentation runtime.
    //   - `uiautomator` — used by `SmokeTest` for hardware-back via
    //     `UiDevice.pressBack()`.
    //   - Paparazzi pulls its own `layoutlib`/`compose-runtime-bridge`
    //     transitively; we only declare the plugin.
    // -----------------------------------------------------------------
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.6.1")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
    androidTestImplementation("androidx.test.uiautomator:uiautomator:2.3.0")
    // `androidx.test.rule.GrantPermissionRule` for pre-granting runtime
    // perms in instrumentation tests (CAMERA, POST_NOTIFICATIONS).
    androidTestImplementation("androidx.test:rules:1.6.1")
    debugImplementation("androidx.compose.ui:ui-tooling")
    debugImplementation("androidx.compose.ui:ui-test-manifest")

    // EncryptedSharedPreferences for the self-session bearer + AppAuth
    // state. Replaces the v0 plain SharedPreferences. Floor pinned to
    // the last stable 1.1.x — 1.1.0-alpha07 ships KeyScheme.AES256_GCM.
    implementation("androidx.security:security-crypto:1.1.0-alpha06")

    // AppAuth-Android — self-session OIDC code flow + PKCE against the
    // user's OHD Storage AS. Mirrors connect/web's `oauth4webapi` shape.
    implementation("net.openid:appauth:0.11.1")

    // The OHDC Kotlin client lands at ../shared/ohdc-clients/kotlin once the
    // storage component publishes its first codegen drop. Wire it as:
    //   implementation(project(":shared:ohdc-clients:kotlin"))
    // or via Maven coords (TBD). Currently absent — Connect Android only
    // exercises the in-process uniffi path, not the remote OHDC wire.

    // Health Connect — `androidx.health.connect:connect-client`.
    //
    // Version `1.1.0-alpha07` is the latest 1.1.x release that still
    // builds against AGP 8.6.x / compileSdk 34 — the 1.1.0-rc / 1.1.0
    // line bumps the AAR-metadata floor to compileSdk 36 and AGP 8.9.x
    // (not yet adopted by this project; bumping AGP is its own commit).
    //
    // Within the alpha07 surface we use the stable `HealthConnectClient`,
    // `PermissionController`, and the read APIs for `StepsRecord`,
    // `HeartRateRecord`, `BloodPressureRecord`, `BloodGlucoseRecord`,
    // `WeightRecord`, `BodyTemperatureRecord`, `SleepSessionRecord`, and
    // `OxygenSaturationRecord`. None of those got renamed between alpha07
    // and the rc, so the migration is a one-line bump when AGP is updated.
    implementation("androidx.health.connect:connect-client:1.1.0-alpha07")

    // Google Code Scanner (Play Services) — fullscreen scanner UI used as a
    // fallback when the user denies our CAMERA permission. The inline
    // preview below is the primary path; this kicks in only when CameraX
    // can't be bound. See https://developers.google.com/ml-kit/code-scanner.
    implementation("com.google.android.gms:play-services-code-scanner:16.1.0")

    // CameraX — embedded camera preview inside `FoodScreen` / `FoodSearchScreen`
    // (the 207 dp scan-area frame). Frames flow through `ImageAnalysis` into
    // ML Kit's barcode detector below. Pinned to 1.4.x — `core-camera2-view`
    // ships the `PreviewView` we expose via `AndroidView`. Bumping to 1.5.x
    // requires compileSdk 35; we're on 34 for now.
    val cameraxVersion = "1.4.1"
    implementation("androidx.camera:camera-core:$cameraxVersion")
    implementation("androidx.camera:camera-camera2:$cameraxVersion")
    implementation("androidx.camera:camera-lifecycle:$cameraxVersion")
    implementation("androidx.camera:camera-view:$cameraxVersion")
    // Guava's `ListenableFuture` is what `ProcessCameraProvider.getInstance`
    // returns. The Android-flavoured 33.x artifact ships the full set of
    // concurrent-futures classes Kotlin needs to call `addListener`. The
    // empty `listenablefuture:1.0` artifact alone wasn't enough — it's a
    // stub used at compile time when full Guava is already present.
    implementation("com.google.guava:guava:33.3.1-android")

    // ML Kit barcode scanning (unbundled / Play-services-backed). Smaller
    // APK footprint than the bundled variant (~3 MB saved) at the cost of
    // a one-time on-device model download via Play Services. The
    // `<meta-data android:name="com.google.mlkit.vision.DEPENDENCIES" />`
    // in the manifest triggers that download on first use.
    implementation("com.google.android.gms:play-services-mlkit-barcode-scanning:18.3.1")

    // WorkManager — backs the periodic Health Connect sync worker
    // (`data/HealthConnectSyncWorker.kt`). 15-min PeriodicWorkRequest is the
    // platform minimum; jobs run via the JobScheduler underneath, so they
    // survive process death + reboot when scheduled with the policy set in
    // `HealthConnectScheduler`.
    implementation("androidx.work:work-runtime-ktx:2.9.1")

    // ML Kit barcode scanning, CameraX, Cronet dependencies will land
    // alongside their corresponding features.
}

// =============================================================================
// Documentation-only Rust core build task.
//
// Real runners would hook this into `preBuild` or generate the `.so` files
// during a CI job; for the scaffolding phase the task only prints the
// command developers should run manually. See
// `connect/android/BUILD.md` for the full recipe.
//
// Rationale for not exec'ing cargo-ndk from Gradle in v0:
//   - The first build pulls bundled-sqlcipher and takes ~3 minutes per ABI
//     (~9 minutes for the default three-ABI set). Hooking that into every
//     `assembleDebug` would make iteration painful.
//   - The Rust workspace is two directories up; resolving the path
//     portably across Windows / macOS / Linux + propagating
//     ANDROID_NDK_HOME deserves its own small build script, not an
//     inlined one-liner.
//   - When CI lands, the canonical recipe is the same as the human one,
//     so we don't want to encode a divergent path in Gradle.
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
              -o ../../../connect/android/app/src/main/jniLibs \
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
        println("      --out-dir ../connect/android/app/src/main/java/uniffi")
        println("== see connect/android/BUILD.md for the full recipe ==")
    }
}
