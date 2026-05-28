package com.ohd.connect

import android.content.Intent
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

    companion object {
        /**
         * Intent extra carrying a nav route to land on after launch. Set by
         * [com.ohd.connect.data.ShareResponderService]'s persistent
         * notification so tapping it opens the app on the Shares screen.
         */
        const val EXTRA_START_ROUTE = "com.ohd.connect.extra.START_ROUTE"

        /**
         * Intent action fired by the App Shortcut + App Actions capability
         * declared in `res/xml/shortcuts.xml`. Logs one `food.eaten` event
         * with `name` / `grams` parsed from the intent extras (defaults
         * applied when the invoker didn't supply them) — see
         * [handleLogFoodIntent].
         */
        const val ACTION_LOG_FOOD = "com.ohd.connect.action.LOG_FOOD"

        /** Intent extra: food name as `String`. */
        const val EXTRA_FOOD_NAME = "name"

        /**
         * Intent extra: amount in grams. Accepts `Double` / `Float` / `Int`,
         * or a `String` that parses as a number — Gemini and `adb am --es`
         * both pass primitives as strings.
         */
        const val EXTRA_FOOD_GRAMS = "grams"
    }

    /** Route the launching intent asked us to land on, if any. */
    private var pendingRoute by mutableStateOf<String?>(null)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        StorageRepository.init(applicationContext)
        pendingRoute = intent?.getStringExtra(EXTRA_START_ROUTE)
        handleLogFoodIntent(intent)
        setContent {
            OhdTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    OhdConnectApp(
                        startRoute = pendingRoute,
                        onStartRouteConsumed = { pendingRoute = null },
                    )
                }
            }
        }
    }

    /**
     * A second tap on the share-responder notification while the activity is
     * already alive arrives here — re-publish the requested route so the
     * Compose tree navigates again.
     */
    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        intent.getStringExtra(EXTRA_START_ROUTE)?.let { pendingRoute = it }
        handleLogFoodIntent(intent)
    }

    /**
     * Gemini / Assistant / App Shortcut → log a food event. Parses
     * [EXTRA_FOOD_NAME] (default "Quick log") and [EXTRA_FOOD_GRAMS]
     * (default 100 g, accepts numeric or string extras) from [intent] and
     * writes one `food.eaten` event with notes `gemini:log_food` so the
     * provenance is queryable.
     *
     * Trigger paths:
     *  - launcher long-press → "Quick-log a food" shortcut (defaults only);
     *  - adb: `am start -a com.ohd.connect.action.LOG_FOOD
     *    -n com.ohd.connect/.MainActivity --es name "banana" --es grams 120`;
     *  - Google Assistant / Gemini matching the
     *    `actions.intent.CREATE_THING` capability declared in
     *    `res/xml/shortcuts.xml` (`thing.name` → `name` extra).
     *
     * Best-effort: runs on a background thread so launch isn't held up;
     * any storage failure is logged and swallowed (the surface only fires
     * for opt-in invocations).
     */
    private fun handleLogFoodIntent(intent: Intent?) {
        if (intent?.action != ACTION_LOG_FOOD) return
        // Capture invocation timestamp + extras synchronously so the row
        // reflects when the invoker called, not when the IO thread runs.
        val ts = System.currentTimeMillis()
        val name = intent.getStringExtra(EXTRA_FOOD_NAME)?.takeIf { it.isNotBlank() }
            ?: "Quick log"
        val grams = parseGramsExtra(intent) ?: 100.0
        Thread {
            runCatching {
                StorageRepository.init(applicationContext)
                val input = com.ohd.connect.data.EventInput(
                    timestampMs = ts,
                    eventType = "food.eaten",
                    channels = listOf(
                        com.ohd.connect.data.EventChannelInput(
                            path = "name",
                            scalar = com.ohd.connect.data.OhdScalar.Text(name),
                        ),
                        com.ohd.connect.data.EventChannelInput(
                            path = "grams",
                            scalar = com.ohd.connect.data.OhdScalar.Real(grams),
                        ),
                    ),
                    notes = "gemini:log_food",
                    topLevel = true,
                )
                val outcome = StorageRepository.putEvent(input).getOrNull()
                android.util.Log.i(
                    "OhdConnect.Gemini",
                    "log_food intent → $outcome (name=$name grams=$grams)",
                )
            }.onFailure { e ->
                android.util.Log.w("OhdConnect.Gemini", "log_food write failed", e)
            }
        }.start()
    }

    /**
     * Coerce the grams extra into a Double. Accepts numeric primitives or
     * strings (`adb am --es` and Assistant always hand strings).
     */
    private fun parseGramsExtra(intent: Intent): Double? {
        if (intent.hasExtra(EXTRA_FOOD_GRAMS)) {
            // Try the most precise types first.
            intent.extras?.let { b ->
                b.getString(EXTRA_FOOD_GRAMS)?.toDoubleOrNull()?.let { return it }
            }
            val asDouble = intent.getDoubleExtra(EXTRA_FOOD_GRAMS, Double.NaN)
            if (!asDouble.isNaN()) return asDouble
            val asFloat = intent.getFloatExtra(EXTRA_FOOD_GRAMS, Float.NaN)
            if (!asFloat.isNaN()) return asFloat.toDouble()
            val asInt = intent.getIntExtra(EXTRA_FOOD_GRAMS, Int.MIN_VALUE)
            if (asInt != Int.MIN_VALUE) return asInt.toDouble()
        }
        return null
    }
}

@Composable
private fun OhdConnectApp(
    startRoute: String? = null,
    onStartRouteConsumed: () -> Unit = {},
) {
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
                // The storage option must be persisted *before* opening so
                // StorageRepository's open-time dispatch (which reads the
                // persisted mode) selects the right backend.
                Auth.saveStorageOption(ctx, selected.name)
                // Remote mode: the OnboardingStorageScreen has already run the
                // OIDC sign-in and persisted the storage URL + self-session
                // token, so the backend is built straight from those — there
                // is no local `.ohd` file or SQLCipher key, and no local
                // self-session token to mint.
                val remote = StorageRepository.isRemoteMode()
                // TODO: real key derivation per spec/encryption.md.
                //       For v0 we use a deterministic stub key so the
                //       SQLCipher PRAGMA key is well-formed.
                val stubKeyHex = if (remote) "" else "00".repeat(32)
                val openResult =
                    if (!remote && StorageRepository.isInitialised()) {
                        StorageRepository.open(stubKeyHex)
                    } else {
                        StorageRepository.openOrCreate(stubKeyHex)
                    }
                openResult
                    .onFailure { e -> setupError = "Storage open failed: ${e.message}" }
                    .onSuccess {
                        // The self-session token is minted by the local core
                        // (`issue_self_session_token`); in remote mode the
                        // bearer token already arrived from the OIDC sign-in.
                        val tokenResult =
                            if (remote) Result.success("")
                            else StorageRepository.issueSelfSessionToken()
                        tokenResult
                            .onFailure { e -> setupError = "Token issue failed: ${e.message}" }
                            .onSuccess {
                                Auth.markFirstRunDone(ctx)
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
                                // Free-tier retention enforcement is an
                                // on-device-only concern — server-hosted plans
                                // manage retention server-side. Skip it in
                                // remote mode.
                                if (!remote) {
                                    com.ohd.connect.data.FreeTierRetentionScheduler.enable(ctx)
                                }
                                if (selected != StorageOption.OnDevice && !remote) {
                                    // A remote option was picked but the OIDC
                                    // sign-in did not complete (no URL/token) —
                                    // we fell back to on-device storage.
                                    setupError =
                                        "Sign-in didn't complete — for now your data " +
                                            "lives on this device. Switch storage later " +
                                            "from Settings."
                                }
                                inSetup = false
                            }
                    }
            },
            errorMessage = setupError,
            onErrorDismiss = { setupError = null },
        )
    } else {
        // Cold-start path. The backend is non-null only when we just came
        // from onboarding (same process). Otherwise we re-open:
        //  - remote mode: build the RemoteStorageBackend straight from the
        //    persisted storage URL + self-session token — there is no local
        //    `.ohd` file or SQLCipher key.
        //  - on-device mode: the data file exists on disk; reopen it with the
        //    stub key (today's behaviour, byte-for-byte).
        val opened = remember {
            when {
                StorageRepository.isOpen() -> Result.success(Unit)
                StorageRepository.isRemoteMode() -> StorageRepository.open(keyHex = "")
                StorageRepository.isInitialised() -> {
                    val stubKeyHex = "00".repeat(32)
                    StorageRepository.open(stubKeyHex)
                }
                else -> Result.failure(IllegalStateException("Storage not initialised"))
            }
        }
        opened.onFailure { /* TODO: route to passphrase reset / re-login */ }

        // Health Connect sync + reminders + the share responder all assume
        // the in-process local storage core (they call into `OhdStorage`
        // directly or host a local relay tunnel). They are on-device-only:
        // gate every cold-start hook to `OnDevice` mode so remote mode never
        // schedules a worker that would dead-end against a remote backend.
        val onDevice = !StorageRepository.isRemoteMode()

        // Apply the persisted Health-Connect auto-sync preference on every
        // cold start. Idempotent — WorkManager keeps the existing schedule
        // via `ExistingPeriodicWorkPolicy.KEEP`.
        LaunchedEffect(Unit) {
            if (onDevice) {
                HealthConnectScheduler.applyPersistedPreference(ctx)
                // Same shape for reminders — enable iff any of the three
                // reminder toggles is on. The worker reads each toggle every
                // tick so user changes don't need a reschedule.
                RemindersScheduler.applyPersistedPreference(ctx)
            }
        }

        // Resume every share the user left with remote access enabled
        // (CORD data link Phase 4d) by starting the durable share-responder
        // foreground service — its onStartCommand re-dials every persisted
        // binding so the relay tunnel comes back up after an app restart.
        // Started iff at least one remote-share binding is persisted; the
        // binding list is the source of truth (it does not need storage
        // open, so there is no cold-start race against the storage handle).
        LaunchedEffect(Unit) {
            kotlinx.coroutines.withContext(kotlinx.coroutines.Dispatchers.IO) {
                // The share responder hosts a *local* relay tunnel over the
                // on-device storage core — there is no responder to resume in
                // remote mode.
                val hasRemoteShare = com.ohd.connect.data.Auth
                    .listRemoteShareGrantUlids(ctx).isNotEmpty()
                if (onDevice && hasRemoteShare) {
                    com.ohd.connect.data.ShareResponderService.start(ctx)
                }
            }
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
                    if (onDevice &&
                        HealthConnectScheduler.isEnabled(ctx) &&
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

        OhdConnectShell(
            startRoute = startRoute,
            onStartRouteConsumed = onStartRouteConsumed,
        )
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
private fun OhdConnectShell(
    startRoute: String? = null,
    onStartRouteConsumed: () -> Unit = {},
) {
    val navController = rememberNavController()
    val snackbar = remember { SnackbarHostState() }

    // A launch route from the share-responder notification ("Shares" screen).
    // Navigated once the NavHost is composed so Home stays on the back stack.
    LaunchedEffect(startRoute) {
        val route = startRoute ?: return@LaunchedEffect
        navController.navigate(route)
        onStartRouteConsumed()
    }

    Scaffold(
        modifier = Modifier.fillMaxSize(),
        snackbarHost = { SnackbarHost(snackbar) },
    ) { padding ->
        OhdNavHost(
            navController = navController,
            contentPadding = padding,
            snackbar = snackbar,
        )

        // Phase 4 — terminal remote-auth failure surface. `SessionState`
        // (an observable flag) flips when any remote storage call fails with
        // a terminal `RemoteAuthException` ("session expired / revoked").
        // We route the user back to the storage picker, which hosts the OIDC
        // re-sign-in flow. Clearing the flag here means a fresh remote call
        // re-raises it if the session is still bad.
        if (com.ohd.connect.data.SessionState.reloginNeeded) {
            androidx.compose.material3.AlertDialog(
                onDismissRequest = { /* deliberate no-op — force a choice */ },
                title = {
                    androidx.compose.material3.Text("Your session expired")
                },
                text = {
                    androidx.compose.material3.Text(
                        "Your remote storage session has expired or was revoked. " +
                            "Sign in again to keep using cloud storage.",
                    )
                },
                confirmButton = {
                    androidx.compose.material3.TextButton(
                        onClick = {
                            com.ohd.connect.data.SessionState.clear()
                            navController.navigate(
                                com.ohd.connect.ui.nav.OhdRoute.SettingsStorage.route,
                            )
                        },
                    ) {
                        androidx.compose.material3.Text("Sign in again")
                    }
                },
                dismissButton = {
                    androidx.compose.material3.TextButton(
                        onClick = { com.ohd.connect.data.SessionState.clear() },
                    ) {
                        androidx.compose.material3.Text("Later")
                    }
                },
            )
        }
    }
}

@Preview(showBackground = true)
@Composable
private fun OhdConnectShellPreview() {
    OhdTheme {
        OhdConnectShell()
    }
}
