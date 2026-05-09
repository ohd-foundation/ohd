plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
    id("org.jetbrains.kotlin.plugin.compose")
}

android {
    namespace = "com.ohd.connect"
    compileSdk = 34

    defaultConfig {
        applicationId = "com.ohd.connect"
        minSdk = 29
        targetSdk = 34
        versionCode = 1
        versionName = "0.0.0"

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

    implementation("androidx.compose.ui:ui")
    implementation("androidx.compose.ui:ui-graphics")
    implementation("androidx.compose.ui:ui-tooling-preview")
    implementation("androidx.compose.material3:material3")
    implementation("androidx.compose.material:material-icons-core")
    implementation("androidx.compose.material:material-icons-extended")

    // JNA (Java Native Access) — uniffi 0.28's Kotlin codegen routes every
    // FFI call through `com.sun.jna.Native.register(...)`. The `@aar`
    // suffix is critical: the plain JAR is missing the per-ABI Android
    // native loader stubs and produces `UnsatisfiedLinkError` at runtime.
    // 5.14+ is the floor; 5.13 doesn't ship a stable Android AAR.
    implementation("net.java.dev.jna:jna:5.14.0@aar")

    // Tests are not wired in the v0 scaffold; left here so the implementation
    // phase has a coordinated dependency floor.
    testImplementation("junit:junit:4.13.2")
    androidTestImplementation("androidx.test.ext:junit:1.2.1")
    androidTestImplementation("androidx.test.espresso:espresso-core:3.6.1")
    androidTestImplementation("androidx.compose.ui:ui-test-junit4")
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

    // Health Connect, ML Kit barcode scanning, CameraX, WorkManager, Cronet
    // dependencies will land in implementation phase per the spec docs in
    // ../spec/health-connect.md, ../spec/barcode-scanning.md, etc.
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
