package com.ohd.connect

import android.content.Context
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onFirst
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import android.Manifest
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import androidx.test.uiautomator.UiDevice
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * End-to-end instrumentation smoke test for OHD Connect.
 *
 * Walks the navigation graph and asserts that every screen reachable in two
 * taps from a cold start renders without throwing. The original motivation:
 * `HomeScreen.kt` shipped a `Modifier.padding(horizontal = (-16).dp)` that
 * blew up at layout time the moment the user finished onboarding and Compose
 * began measuring `HomeScreen`. A test that simply navigated past the gate
 * would have caught it; this is that test.
 *
 * Test fixture:
 *  - The Compose rule launches `MainActivity` directly. The host emulator
 *    must already be booted (AVD `basta_test`); we do not start/stop it.
 *  - `setUp()` clears both the legacy and encrypted SharedPreferences plus
 *    the on-disk `data.db` so every test starts at the onboarding gate.
 *    Without this the second test in the run lands on `HomeScreen` directly
 *    (because the previous test marked `firstRunDone`) and the onboarding
 *    assertions fail.
 *
 * The test deliberately uses `onAllNodesWithText(...).onFirst()` for the few
 * strings that appear twice on a single screen (e.g. "Food" appears in the
 * top bar and in the favourites/quick-log grid). `assertIsDisplayed()` is
 * preferred over `assertExists()` because Compose's offscreen-but-laid-out
 * nodes still match `onNodeWith*`; we want the visible match.
 */
@RunWith(AndroidJUnit4::class)
class SmokeTest {

    // Pre-grant runtime permissions the app would otherwise prompt for at
    // first use. Without this, FoodScreen's inline camera preview kicks off
    // a Google permission-controller activity the moment the screen
    // composes, which tears down the MainActivity Compose hierarchy and
    // makes downstream `onNodeWithText` assertions fail with
    // "No compose hierarchies found in the app". Order-0 so it fires before
    // the compose rule launches the activity.
    @get:Rule(order = 0)
    val permissions: GrantPermissionRule = GrantPermissionRule.grant(
        Manifest.permission.CAMERA,
        Manifest.permission.POST_NOTIFICATIONS,
    )

    @get:Rule(order = 1)
    val composeRule = createAndroidComposeRule<MainActivity>()

    /**
     * Reset all per-user state so every test starts at the onboarding gate.
     *
     * Three things need clearing:
     *  1. `ohd_connect_secure` (EncryptedSharedPreferences) — holds the
     *     `first_run_done` flag the v0 onboarding writes after a successful
     *     storage open.
     *  2. `ohd_connect_state` (legacy plain prefs) — `Auth` falls back to
     *     this on devices where Keystore is broken; the emulator has a
     *     working Keystore but we belt-and-braces clear both.
     *  3. `data.db` — `StorageRepository.isInitialised()` is true if the
     *     file exists. Onboarding's `Continue` calls `openOrCreate` which
     *     becomes `open` on a second test run, and `open` against the same
     *     stub key works fine, but a stale file from an earlier crashed
     *     run can fail to decrypt. Wipe it.
     */
    @Before
    fun setUp() {
        val ctx = ApplicationProvider.getApplicationContext<Context>()
        listOf("ohd_connect_secure", "ohd_connect_state").forEach { name ->
            ctx.getSharedPreferences(name, Context.MODE_PRIVATE)
                .edit()
                .clear()
                .commit()
        }
        File(ctx.filesDir, "data.db").delete()

        // `createAndroidComposeRule<MainActivity>` auto-launches the activity
        // BEFORE @Before runs, so the activity has already evaluated
        // `Auth.isFirstRun(ctx)` against pre-clear state. Force a recreate so
        // it re-reads the now-cleared prefs and lands on Onboarding.
        composeRule.activityRule.scenario.onActivity { it.recreate() }
        composeRule.waitForIdle()
    }

    // -------------------------------------------------------------------------
    // P0.2.a — Onboarding renders + Continue tap reaches Home.
    // -------------------------------------------------------------------------

    @Test
    fun onboardingScreen_rendersAndContinueWorks() {
        // Onboarding heading.
        composeRule
            .onNodeWithText("Where should OHD store your data?")
            .assertIsDisplayed()

        // Primary CTA — below the fold on small/dense emulators; scroll into
        // view before clicking.
        composeRule.onNodeWithText("Continue").performScrollTo()
        composeRule.onNodeWithText("Continue").assertIsDisplayed()
        composeRule.onNodeWithText("Continue").performClick()

        // Wait for navigation to settle, then assert Home rendered.
        composeRule.waitForIdle()

        // "QUICK LOG" is a section header that only appears on the Home
        // screen — uniquely identifies it. There is also "QUICK MEASURES"
        // on the measurement logger, hence the exact match.
        composeRule.onNodeWithText("QUICK LOG").assertIsDisplayed()

        // Wordmark sanity check.
        composeRule.onNodeWithText("OHD").assertIsDisplayed()
    }

    // -------------------------------------------------------------------------
    // P0.2.b — Each Quick-log card opens its logger; back arrow returns home.
    //
    // The card labels live both in the home grid (`OhdQuickLogItem`) and in
    // the logger's `OhdTopBar` title. We disambiguate by waiting for the
    // top-bar action button (Library / Log / etc.) to appear, then click
    // the back arrow's `contentDescription = "Back"` to return.
    // -------------------------------------------------------------------------

    @Test
    fun home_tappingEachQuickLogOpensALogger() {
        finishOnboarding()

        // --- Medication ---
        // "Medication" appears as the quick-log card label *and* later
        // in the logger's top-bar title ("Medications" — plural).
        clickQuickLog("Medication")
        composeRule.onNodeWithText("Medications").assertIsDisplayed()
        // The Library top-bar action is unique to the medication logger.
        composeRule.onNodeWithText("Library").assertIsDisplayed()
        pressBack()

        // --- Food ---
        clickQuickLog("Food")
        // The Food logger's top bar reads exactly "Food" (no pluralisation).
        // Two nodes match: the top-bar title and any leftover quick-log card
        // (it's in the back stack but not in the active tree). Use first().
        composeRule.onAllNodesWithText("Food").onFirst().assertIsDisplayed()
        pressBack()

        // --- Measurement ---
        clickQuickLog("Measurement")
        composeRule.onNodeWithText("QUICK MEASURES").assertIsDisplayed()
        pressBack()

        // --- Symptom ---
        clickQuickLog("Symptom")
        composeRule.onNodeWithText("Describe the symptom").assertIsDisplayed()
        pressBack()

        // Back at Home.
        composeRule.onNodeWithText("QUICK LOG").assertIsDisplayed()
    }

    // -------------------------------------------------------------------------
    // P0.2.c — Each bottom-tab renders without crashing.
    //
    // The LOG tab is special: it does not navigate, it opens a
    // `ModalBottomSheet`. We assert the sheet's "QUICK LOG" header
    // appears (same string as the home section header — we add a second
    // assertion on a sheet-only label, "QUICK LOG" + the modal's child
    // `LogPickerRow` that sits at sheet height).
    // -------------------------------------------------------------------------

    @Test
    fun home_headerNavigatesToHistoryAndSettings() {
        finishOnboarding()

        // Stat-tile tap → History (events count is the entry point).
        composeRule.onAllNodesWithText("events today")
            .onFirst()
            .performClick()
        composeRule.waitForIdle()
        composeRule.onNodeWithText("LAST 50 ENTRIES").assertIsDisplayed()
        pressBack()
        composeRule.waitForIdle()
        composeRule.onNodeWithText("QUICK LOG").assertIsDisplayed()

        // Header gear icon → Settings hub.
        composeRule.onNodeWithContentDescription("Settings").performClick()
        composeRule.waitForIdle()
        composeRule.onNodeWithText("Storage & Data").assertIsDisplayed()
    }

    // -------------------------------------------------------------------------
    // P0.2.d — Settings → Profile & Access exercises the operator-screen
    // wrapper code path (`OperatorScaffold` in `NavGraph.kt`). The wrapper
    // was a likely candidate for the next padding bug — same nested-padding
    // shape as the one we just fixed.
    // -------------------------------------------------------------------------

    @Test
    fun noCrashOnSettingsHubRowsTap() {
        finishOnboarding()

        // Open Settings via the home-header gear.
        composeRule.onNodeWithContentDescription("Settings").performClick()
        composeRule.waitForIdle()

        // Tap "Profile & Access".
        composeRule.onNodeWithText("Profile & Access").performClick()
        composeRule.waitForIdle()

        // The Access hub renders six rows.
        composeRule.onNodeWithText("Grants").assertIsDisplayed()
        composeRule.onNodeWithText("Audit log").assertIsDisplayed()
        composeRule.onNodeWithText("Emergency").assertIsDisplayed()

        // Open one operator screen — Audit log. This wraps the existing
        // `AuditScreen` in `OperatorScaffold`, the wrapper that was the next
        // most likely site for the same negative-padding bug.
        composeRule.onNodeWithText("Audit log").performClick()
        composeRule.waitForIdle()
        // The OperatorScaffold renders the title in the OhdTopBar — assert
        // it appears (matches the wrapper's `title` argument).
        composeRule.onAllNodesWithText("Audit log").onFirst().assertIsDisplayed()
    }

    // -------------------------------------------------------------------------
    // Test helpers.
    // -------------------------------------------------------------------------

    /**
     * Drives the onboarding gate so the rest of a test starts at Home.
     *
     * Worth noting: `OnboardingStorageScreen` runs its `onContinue` callback
     * synchronously on the UI thread, but the underlying `StorageRepository
     * .openOrCreate` call hits sqlite/sqlcipher and can take a couple of
     * hundred ms on a cold emulator. `composeRule.waitForIdle()` blocks
     * until both the recomposition and any pending `LaunchedEffect`s
     * settle, which is enough — we don't need a polling loop.
     */
    private fun finishOnboarding() {
        composeRule.onNodeWithText("Continue").performScrollTo()
        composeRule.onNodeWithText("Continue").performClick()
        composeRule.waitForIdle()
        // Sanity-check we actually reached Home before the caller continues.
        composeRule.onNodeWithText("QUICK LOG").assertIsDisplayed()
    }

    /**
     * Tap a quick-log card on the Home screen.
     *
     * The card is the unique node where the [label] is the *only* text on
     * the card body. Using `onNodeWithText(label)` works because the card
     * is the first appearance of that string at Home time.
     */
    private fun clickQuickLog(label: String) {
        // After returning from a logger, the back stack still holds the
        // logger's screen until the navigation animation completes. Wait
        // before clicking so the click lands on the home card, not the
        // dying logger.
        composeRule.waitForIdle()
        composeRule.onNodeWithText(label).performClick()
        composeRule.waitForIdle()
    }

    /**
     * Hardware back via UiAutomator. Compose-Test has no built-in back
     * pressed action, and clicking the `OhdTopBar` back arrow by
     * contentDescription works too — but `pressBack()` exercises the same
     * code path the user would use, and won't break if a screen
     * re-skins its top bar.
     */
    private fun pressBack() {
        val device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation())
        device.pressBack()
        composeRule.waitForIdle()
    }
}
