package com.ohd.connect.ui.screens.settings

import android.widget.Toast
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.Auth
import com.ohd.connect.ui.components.OhdCard
import com.ohd.connect.ui.components.OhdToggle
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Reminders & Calendar settings — Pencil `VCokI` "Reminders & Calendar" panel.
 *
 * Three cards stacked:
 *  1. **Medication reminders** — single toggle. Default on. Drives the
 *     dose-due notifications fired by the notification engine.
 *  2. **Daily summary** — toggle + time chip (chip is a stub for v1; the
 *     time picker dialog isn't shipped yet).
 *  3. **Calendar export** — toggle that mirrors med doses to the user's
 *     default phone calendar. Off by default. The actual calendar write
 *     path is the notification engine's responsibility.
 *
 * This screen owns no execution — it just persists the three prefs back
 * to Auth on every toggle. The sibling notification-engine agent polls
 * those same prefs when deciding what to fire.
 */
@Composable
fun RemindersSettingsScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
) {
    val ctx = LocalContext.current

    var medsEnabled by remember { mutableStateOf(Auth.medsRemindersEnabled(ctx)) }
    var dailyEnabled by remember { mutableStateOf(Auth.dailySummaryEnabled(ctx)) }
    var calendarEnabled by remember { mutableStateOf(Auth.calendarExportEnabled(ctx)) }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Reminders & Calendar", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            // ---- 1. Medication reminders -----------------------------------
            OhdCard(title = "Medication reminders") {
                ToggleRow(
                    title = "Notify when prescribed meds are due",
                    body = "Uses your prescribed schedule from the medication list. Notifications fire when a dose is due and haven't been logged within the last hour.",
                    checked = medsEnabled,
                    onCheckedChange = { v ->
                        medsEnabled = v
                        Auth.setMedsRemindersEnabled(ctx, v)
                    },
                )
            }

            // ---- 2. Daily summary ------------------------------------------
            OhdCard(title = "Daily summary") {
                ToggleRow(
                    title = "Daily summary notification",
                    body = "One notification per day with totals: meds taken, foods logged, latest vitals.",
                    checked = dailyEnabled,
                    onCheckedChange = { v ->
                        dailyEnabled = v
                        Auth.setDailySummaryEnabled(ctx, v)
                    },
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(
                        text = "Deliver at",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 13.sp,
                        color = OhdColors.Ink,
                        modifier = Modifier.weight(1f),
                    )
                    TimeChip(
                        label = "9:00 PM",
                        onClick = {
                            Toast.makeText(
                                ctx,
                                "Time picker coming soon",
                                Toast.LENGTH_SHORT,
                            ).show()
                        },
                    )
                }
            }

            // ---- 3. Calendar export ----------------------------------------
            OhdCard(title = "Calendar export") {
                ToggleRow(
                    title = "Add medication reminders to phone calendar",
                    body = "Adds an event per scheduled dose to your default calendar so reminders fire on your watch / Pixel without OHD running.",
                    checked = calendarEnabled,
                    onCheckedChange = { v ->
                        calendarEnabled = v
                        Auth.setCalendarExportEnabled(ctx, v)
                    },
                )
            }
        }
    }
}

/**
 * Title + body label paired with an [OhdToggle] on the right. Mirrors the
 * pattern used in HealthConnect → Auto-sync; pulled out here because all
 * three Reminders cards share it.
 */
@Composable
private fun ToggleRow(
    title: String,
    body: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(
                text = title,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 14.sp,
                color = OhdColors.Ink,
            )
            Text(
                text = body,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                lineHeight = 18.sp,
                color = OhdColors.Muted,
            )
        }
        OhdToggle(
            checked = checked,
            onCheckedChange = onCheckedChange,
        )
    }
}

/**
 * "9:00 PM ▾" style chip — same visual treatment as `RetentionChip` in
 * StorageSettingsScreen, but tap fires the supplied [onClick] (v1: a
 * `Toast` stub for the not-yet-shipped time picker).
 */
@Composable
private fun TimeChip(
    label: String,
    onClick: () -> Unit,
) {
    val shape = RoundedCornerShape(4.dp)
    Row(
        modifier = Modifier
            .background(OhdColors.Bg, shape)
            .border(BorderStroke(1.dp, OhdColors.Line), shape)
            .clickable { onClick() }
            .padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Ink,
        )
    }
}
