package com.ohd.connect

import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.ui.unit.dp
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.tooling.preview.Preview
import androidx.navigation.compose.rememberNavController
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import com.ohd.connect.data.Auth
import com.ohd.connect.data.HealthConnectScheduler
import com.ohd.connect.data.OhdAccountStore
import com.ohd.connect.data.OhdHealthConnect
import com.ohd.connect.data.RemindersScheduler
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.data.syncFromHealthConnect
import kotlinx.coroutines.delay
import com.ohd.connect.ui.nav.OhdNavHost
import com.ohd.connect.ui.screens.OnboardingStorageScreen
import com.ohd.connect.ui.screens._shared.StorageOption
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdTheme

/**
 * Single-Activity entry point for OHD Connect.
 *
 * Flow:
 *   - First launch: [Auth.isFirstRun] is true → render [SetupScreen]
 *     (option (a) in the migration brief — keep the existing first-run
 *     gate; the new [com.ohd.connect.ui.screens.OnboardingStorageScreen]
 *     ships only as a visual variant reachable from
 *     [OhdRoute.OnboardingStorage]). Setup completion flips a remembered
 *     state so we drop into [OhdConnectShell] without restarting.
 *   - Subsequent launches: open the existing storage on-the-fly and render
 *     [OhdConnectShell] — Scaffold around an [OhdNavHost] with the
 *     four-tab [OhdBottomTabBar] gated to root routes.
 *
 * The Compose tree references the uniffi bindings only via
 * [StorageRepository] — none of the screens import `uniffi.ohd_storage.*`.
 * That keeps the rest of the codebase compilable when the Stage 1 / Stage 2
 * codegen flow in `BUILD.md` hasn't been run yet (only
 * `data/StorageRepository.kt` fails to resolve in that case, and even
 * those failures are gated behind TODO comments).
 */
class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        StorageRepository.init(applicationContext)
        setContent {
            OhdTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    OhdConnectApp()
                }
            }
        }
    }
}

@Composable
private fun OhdConnectApp() {
    val ctx = LocalContext.current
    var inSetup by remember { mutableStateOf(Auth.isFirstRun(ctx)) }
    var inClaim by remember { mutableStateOf(false) }
    var setupError by remember { mutableStateOf<String?>(null) }

    if (inClaim) {
        com.ohd.connect.ui.screens.ClaimAccountScreen(
            contentPadding = PaddingValues(0.dp),
            onBack = { inClaim = false },
            onClaimed = { _ ->
                // Recovery succeeded — the access token is persisted.
                // We DON'T overwrite the local profile_ulid here: the next
                // call to `me` (post-setup) syncs the canonical profile.
                inClaim = false
            },
        )
    } else if (inSetup) {
        // Pencil-matched first-run gate (eKtkU). For v1 only the on-device
        // option actually wires storage; the other choices are accepted
        // but mapped to on-device behaviour, with a notice via setupError.
        OnboardingStorageScreen(
            onClaimExistingAccount = { inClaim = true },
            onContinue = { selected ->
                // TODO: real key derivation per spec/encryption.md.
                //       For v0 we use a deterministic stub key so the
                //       SQLCipher PRAGMA key is well-formed.
                val stubKeyHex = "00".repeat(32)
                val openResult =
                    if (StorageRepository.isInitialised()) {
                        StorageRepository.open(stubKeyHex)
                    } else {
                        StorageRepository.openOrCreate(stubKeyHex)
                    }
                openResult
                    .onFailure { e -> setupError = "Storage open failed: ${e.message}" }
                    .onSuccess {
                        StorageRepository.issueSelfSessionToken()
                            .onFailure { e -> setupError = "Token issue failed: ${e.message}" }
                            .onSuccess {
                                Auth.markFirstRunDone(ctx)
                                Auth.saveStorageOption(ctx, selected.name)
                                // Mint the local OHD account + recovery
                                // code on the very first successful setup.
                                // Idempotent: a second call simply re-reads
                                // the persisted row.
                                if (OhdAccountStore.load(ctx) == null) {
                                    val acct = OhdAccountStore.mintFree(ctx)
                                    com.ohd.connect.data.NotificationCenter.append(
                                        ctx,
                                        com.ohd.connect.data.NotificationCenter.NotificationEntry(
                                            id = "ohd_account_recovery_save",
                                            timestampMs = System.currentTimeMillis(),
                                            title = "Save your recovery code",
                                            body = "Settings → Profile & Access → Recovery code. " +
                                                "Without it, losing this device means losing the account.",
                                            kind = com.ohd.connect.data.NotificationCenter.Kind.TEST,
                                            actionRoute = "settings/profile/recovery",
                                        ),
                                    )
                                    // Best-effort sync to api.ohd.dev — keeps `recover` working
                                    // from another device. Network failures are silently
                                    // swallowed; the local mint already succeeded.
                                    com.ohd.connect.data.OhdSaasRegistrar.fireAndForget(ctx, acct)
                                }
                                com.ohd.connect.data.FreeTierRetentionScheduler.enable(ctx)
                                if (selected != StorageOption.OnDevice) {
                                    setupError =
                                        "Cloud / self-hosted / provider hosting are coming soon — " +
                                            "for now your data lives on this device."
                                }
                                inSetup = false
                            }
                    }
            },
            errorMessage = setupError,
            onErrorDismiss = { setupError = null },
        )
    } else {
        // Cold-start path. The handle is non-null only when we just came
        // from onboarding (same process). Otherwise the data file exists
        // on disk and we need to reopen it.
        val opened = remember {
            when {
                StorageRepository.isOpen() -> Result.success(Unit)
                StorageRepository.isInitialised() -> {
                    val stubKeyHex = "00".repeat(32)
                    StorageRepository.open(stubKeyHex)
                }
                else -> Result.failure(IllegalStateException("Storage not initialised"))
            }
        }
        opened.onFailure { /* TODO: route to passphrase reset */ }

        // Apply the persisted Health-Connect auto-sync preference on every
        // cold start. Idempotent — WorkManager keeps the existing schedule
        // via `ExistingPeriodicWorkPolicy.KEEP`.
        LaunchedEffect(Unit) {
            HealthConnectScheduler.applyPersistedPreference(ctx)
            // Same shape for reminders — enable iff any of the three
            // reminder toggles is on. The worker reads each toggle every
            // tick so user changes don't need a reschedule.
            RemindersScheduler.applyPersistedPreference(ctx)
        }

        // Foreground "near-real-time" sync. While the app is RESUMED (user
        // is actively looking at it), pull from Health Connect every 30 s
        // so newly-recorded events on the watch / scale appear in History
        // and the Home stat tile without waiting for the 15-min worker.
        //
        // CRITICAL: must run on Dispatchers.IO. The sync calls into uniffi's
        // `put_event` per record, which is a synchronous blocking call
        // through JNI. On a 5-year backfill from a Galaxy Watch that's tens
        // of thousands of events and would freeze the UI thread for minutes.
        val lifecycleOwner = LocalLifecycleOwner.current
        LaunchedEffect(lifecycleOwner) {
            lifecycleOwner.lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
                while (true) {
                    if (HealthConnectScheduler.isEnabled(ctx) &&
                        StorageRepository.isOpen() &&
                        OhdHealthConnect.availability(ctx) ==
                            OhdHealthConnect.Availability.Installed
                    ) {
                        runCatching {
                            kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
                                syncFromHealthConnect(ctx)
                            }
                        }
                    }
                    delay(30_000L)
                }
            }
        }

        OhdConnectShell()
    }
}

/**
 * Main shell: nav host + snackbar host. No bottom-tab bar.
 *
 * Per Pencil v2 / user feedback: HOME is the default landing surface, settings
 * gear lives in the home header (next to the bell), History is reached by
 * tapping the events stat tile, and each Quick-log card jumps directly to the
 * matching logger — so the bottom bar (with HOME / LOG / HISTORY / SETTINGS)
 * was redundant and is removed. The legacy [OhdBottomTabBar] composable is
 * retained for now in case we want a tablet/desktop layout later.
 */
@Composable
private fun OhdConnectShell() {
    val navController = rememberNavController()
    val snackbar = remember { SnackbarHostState() }

    Scaffold(
        modifier = Modifier.fillMaxSize(),
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        OhdNavHost(
            navController = navController,
            contentPadding = padding,
            snackbar = snackbar,
        )
    }
}

@Preview(showBackground = true)
@Composable
private fun OhdConnectShellPreview() {
    OhdTheme {
        OhdConnectShell()
    }
}
