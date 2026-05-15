package com.ohd.connect

import android.content.Context
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.core.app.ApplicationProvider
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * Compose test for the on-device storage card's retention-limit dialog.
 *
 * The fixture mirrors `SmokeTest`: clears prefs + `data.db` so each test
 * starts at the onboarding gate, then the test taps the live "Unlimited
 * ▾" chip on the on-device card to open the dialog.
 *
 * Coverage:
 *  1. Dialog opens on chip tap.
 *  2. Picking "2 years" + "5 GB" + Apply persists, dismisses, and updates
 *     the chip label to "5 GB · 2 years ▾".
 */
@RunWith(AndroidJUnit4::class)
class RetentionDialogTest {

    @get:Rule
    val composeRule = createAndroidComposeRule<MainActivity>()

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

        composeRule.activityRule.scenario.onActivity { it.recreate() }
        composeRule.waitForIdle()
    }

    @Test
    fun retentionDialog_pickAndApplyUpdatesChipLabel() {
        // Onboarding heading rendered → On-device card is the default
        // selection so its expanded panel is visible.
        composeRule
            .onNodeWithText("Where should OHD store your data?")
            .assertIsDisplayed()

        // The "Unlimited ▾" chip — the dialog's only entry point.
        composeRule.onNodeWithText("Unlimited ▾").performScrollTo()
        composeRule.onNodeWithText("Unlimited ▾").performClick()
        composeRule.waitForIdle()

        // Dialog title.
        composeRule.onNodeWithText("Retention limits").assertIsDisplayed()

        // Pick chips. (The dialog has its own "Unlimited" chip per row;
        // we want the named presets, which are unique.)
        composeRule.onNodeWithText("2 years").performClick()
        composeRule.onNodeWithText("5 GB").performClick()
        composeRule.waitForIdle()

        // Apply.
        composeRule.onNodeWithText("Apply").performClick()
        composeRule.waitForIdle()

        // Chip label now reflects the chosen pair.
        composeRule.onNodeWithText("5 GB · 2 years ▾").assertIsDisplayed()
    }
}
