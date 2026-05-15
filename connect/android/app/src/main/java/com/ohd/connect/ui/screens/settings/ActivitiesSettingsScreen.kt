package com.ohd.connect.ui.screens.settings

import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.EventFilter
import com.ohd.connect.data.HEALTH_CONNECT_TYPES
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Activities settings — Pencil `VCokI` "Activities" panel.
 *
 * Two sections:
 *  1. **Connected sources** — single-row list (Health Connect) with a
 *     "Sync now" right-side affordance that navigates to
 *     `OhdRoute.SettingsHealthConnect` via [onOpenHealthConnect].
 *  2. **What we track** — the same eight record types as the Health
 *     Connect screen, each annotated with its lifetime count from
 *     `StorageRepository.countEvents`. Tap is a no-op for v1.
 *
 * [onOpenHealthConnect] defaults to a noop so existing call sites that
 * don't wire it (the current NavGraph) compile cleanly. The route is
 * already declared at `OhdRoute.SettingsHealthConnect` — the navgraph
 * agent can plug `navController.navigate(...)` whenever they touch it.
 */
@Composable
fun ActivitiesSettingsScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onOpenHealthConnect: () -> Unit = {},
) {
    val ctx = LocalContext.current

    // Per-type counts. Loaded on first composition; we just dispatch one
    // countEvents per type — they're SQL COUNT(*)s and cheap.
    val counts = remember { mutableStateMapOf<String, Long>() }
    LaunchedEffect(Unit) {
        HEALTH_CONNECT_TYPES.forEach { (_, eventType) ->
            val n = StorageRepository
                .countEvents(EventFilter(eventTypesIn = listOf(eventType), limit = null))
                .getOrNull() ?: 0L
            counts[eventType] = n
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Activities", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState()),
        ) {
            // ---- 1. Connected sources --------------------------------------
            OhdSectionHeader(text = "CONNECTED SOURCES")
            ConnectedSourceRow(
                label = "Health Connect",
                secondary = "Reads steps, heart rate, sleep + 5 others",
                actionLabel = "Sync now",
                onAction = onOpenHealthConnect,
                onClick = {
                    // Tapping the row mirrors the action — both reach the
                    // same Health Connect settings sub-screen.
                    onOpenHealthConnect()
                },
            )

            // ---- 2. What we track ------------------------------------------
            OhdSectionHeader(text = "RECORD TYPES")
            HEALTH_CONNECT_TYPES.forEach { (label, eventType) ->
                val n = counts[eventType]
                OhdListItem(
                    primary = label,
                    meta = n?.let { "$it" } ?: "—",
                    onClick = {
                        // No-op for v1. Future: open a per-type history.
                        Toast.makeText(
                            ctx,
                            "Per-type history coming soon",
                            Toast.LENGTH_SHORT,
                        ).show()
                    },
                )
            }
        }
    }
}

/**
 * Custom row for the **Connected sources** list because we want a
 * right-side action label (Inter 14 / 500 / ohd-red, à la a TopBar action)
 * rather than the muted chevron meta that [OhdListItem] gives us. Keeps
 * the visual rhythm of the screen consistent with the rest of the design
 * system (16 dp padding, 1 dp soft separator).
 */
@Composable
private fun ConnectedSourceRow(
    label: String,
    secondary: String,
    actionLabel: String,
    onAction: () -> Unit,
    onClick: () -> Unit,
) {
    Column {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(OhdColors.Bg)
                .clickable { onClick() }
                .padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Box(
                modifier = Modifier.size(24.dp),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    imageVector = OhdIcons.Activity,
                    contentDescription = null,
                    tint = OhdColors.Ink,
                    modifier = Modifier.size(20.dp),
                )
            }
            Column(
                modifier = Modifier.weight(1f),
                verticalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Text(
                    text = label,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 14.sp,
                    color = OhdColors.Ink,
                )
                Text(
                    text = secondary,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
            }
            Box(
                modifier = Modifier
                    .clickable { onAction() }
                    .padding(horizontal = 4.dp, vertical = 4.dp),
            ) {
                Text(
                    text = actionLabel,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 14.sp,
                    color = OhdColors.Red,
                )
            }
        }
    }
}
