package com.ohd.connect.ui.screens

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import com.ohd.connect.data.Auth
import com.ohd.connect.data.EventFilter
import com.ohd.connect.data.EventVisibility
import com.ohd.connect.data.StorageRepository
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import java.util.Calendar
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.components.OhdQuickLogItem
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdSegmentedTimeRange
import com.ohd.connect.ui.components.OhdStatTile
import com.ohd.connect.ui.components.TimeRange
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.screens._shared.AddFavouriteSheet
import com.ohd.connect.ui.screens._shared.HomeFavourite
import com.ohd.connect.ui.screens._shared.decodeHomeFavourites
import com.ohd.connect.ui.screens._shared.encodeHomeFavourites
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdDisplay

/**
 * Home v2 — Pencil `KADlx`, spec §4.1.
 *
 * Custom header (logo + sparkles + bell) instead of [com.ohd.connect.ui.components.OhdTopBar],
 * then a vertically-stacked body with: time-range selector, 2× stat tile, quick-log
 * grid, and favourites strip.
 *
 * The screen takes [contentPadding] from the parent Scaffold so the bottom-tab-bar
 * doesn't overlap it. All navigation is via callback parameters — wiring lives in
 * the navigation agent.
 */
@Composable
fun HomeScreen(
    contentPadding: PaddingValues,
    onOpenCord: () -> Unit,
    onOpenNotifications: () -> Unit,
    onOpenSettings: () -> Unit,
    onOpenHistory: () -> Unit,
    onLogMedication: () -> Unit,
    onLogFood: () -> Unit,
    onLogMeasurement: () -> Unit,
    onLogSymptom: () -> Unit,
    onOpenDevices: () -> Unit,
    onFavouriteClick: (label: String, kind: String) -> Unit = { _, _ -> },
) {
    val ctx = LocalContext.current
    var range by remember { mutableStateOf(TimeRange.Today) }
    var eventCount by remember { mutableLongStateOf(0L) }

    // Favourites — read the persisted blob lazily via [LaunchedEffect] so
    // EncryptedSharedPreferences's first-touch latency (~hundreds of ms on
    // fresh emulators) doesn't stall the initial composition. The default
    // pair (Glucose + Blood pressure) renders immediately; the persisted
    // list (when non-empty) replaces it on the next frame.
    var favourites by remember { mutableStateOf(DEFAULT_FAVOURITES) }
    var addFavouriteSheetOpen by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) {
        // Auth.homeFavouritesJson hits EncryptedSharedPreferences which on
        // first touch can do KeyStore unwrap work — push to IO so the main
        // thread doesn't stall while the rest of the screen composes.
        val loaded = withContext(Dispatchers.IO) {
            decodeHomeFavourites(Auth.homeFavouritesJson(ctx))
        }
        if (loaded.isNotEmpty()) {
            favourites = loaded
        }
    }

    // Re-poll the count while the screen is RESUMED. Triggers immediately on
    // navigate-to-home (good — fresh number on tab return) and every ~5 s
    // while visible so background HC sync writes show up without a manual
    // refresh. The loop stops when the screen is paused (battery-friendly).
    val lifecycleOwner = LocalLifecycleOwner.current
    LaunchedEffect(range, lifecycleOwner) {
        lifecycleOwner.lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            while (true) {
                eventCount = countEventsSince(rangeStartMs(range))
                delay(5_000L)
            }
        }
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding)
            .verticalScroll(rememberScrollState()),
    ) {
        HomeHeader(
            onOpenCord = onOpenCord,
            onOpenNotifications = onOpenNotifications,
            onOpenSettings = onOpenSettings,
        )

        // Top inset block: time-range + stat-tiles (need 16dp horizontal inset).
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(top = 4.dp, start = 16.dp, end = 16.dp),
            verticalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            OhdSegmentedTimeRange(selected = range, onSelect = { range = it })

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                OhdStatTile(
                    value = formatCount(eventCount),
                    label = "events ${rangeLabel(range)}",
                    modifier = Modifier
                        .weight(1f)
                        .clickable { onOpenHistory() },
                )
                OhdStatTile(
                    value = "1",
                    label = "source",
                    modifier = Modifier
                        .weight(1f)
                        .clickable { onOpenDevices() },
                )
            }
        }

        Spacer(Modifier.height(20.dp))

        // Edge-to-edge section header (it owns its own horizontal padding).
        OhdSectionHeader("QUICK LOG")

        // Quick-log grid (2×2) — needs its own 16dp horizontal inset.
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Box(modifier = Modifier.weight(1f)) {
                    OhdQuickLogItem(
                        label = "Medication",
                        icon = OhdIcons.Pill,
                        onClick = onLogMedication,
                    )
                }
                Box(modifier = Modifier.weight(1f)) {
                    OhdQuickLogItem(
                        label = "Food",
                        icon = OhdIcons.Utensils,
                        onClick = onLogFood,
                    )
                }
            }
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Box(modifier = Modifier.weight(1f)) {
                    OhdQuickLogItem(
                        label = "Measurement",
                        icon = OhdIcons.Activity,
                        onClick = onLogMeasurement,
                    )
                }
                Box(modifier = Modifier.weight(1f)) {
                    OhdQuickLogItem(
                        label = "Symptom",
                        icon = OhdIcons.Thermometer,
                        onClick = onLogSymptom,
                    )
                }
            }
        }

        Spacer(Modifier.height(20.dp))

        // Favourites strip — header + chip row, both inset.
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            FavouritesHeader(onAddFavourite = { addFavouriteSheetOpen = true })

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                favourites.forEach { fav ->
                    FavouriteChip(
                        label = fav.label,
                        icon = fav.resolveIcon(),
                        onClick = { onFavouriteClick(fav.label, fav.kind) },
                    )
                }
            }
        }

        Spacer(Modifier.height(16.dp))
    }

    // Add-favourite bottom sheet. Selecting a preset (or submitting a
    // custom label) appends the entry to `home_favourites_v1` in Auth and
    // updates the in-memory list so the chip appears immediately.
    if (addFavouriteSheetOpen) {
        AddFavouriteSheet(
            onDismiss = { addFavouriteSheetOpen = false },
            onPick = { label, kind, iconKey ->
                val entry = HomeFavourite(label = label, kind = kind, iconKey = iconKey)
                val updated = favourites + entry
                favourites = updated
                Auth.saveHomeFavouritesJson(ctx, encodeHomeFavourites(updated))
            },
        )
    }
}

/**
 * Default favourites — Glucose + Blood pressure. Used when the persisted
 * `home_favourites_v1` blob is empty. Once the user pins anything via the
 * "+ Add" sheet the persisted list becomes authoritative.
 */
private val DEFAULT_FAVOURITES: List<HomeFavourite> = listOf(
    HomeFavourite(label = "Glucose", kind = "glucose", iconKey = "droplets"),
    HomeFavourite(label = "Blood pressure", kind = "blood_pressure", iconKey = "heart_pulse"),
)

/** OHD logo + sparkles + bell + settings strip (Pencil `l3AI7` extended). */
@Composable
private fun HomeHeader(
    onOpenCord: () -> Unit,
    onOpenNotifications: () -> Unit,
    onOpenSettings: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = 16.dp, bottom = 8.dp, start = 20.dp, end = 20.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = "OHD",
            fontFamily = OhdDisplay,
            fontWeight = FontWeight.W200,
            fontSize = 28.sp,
            letterSpacing = 1.sp,
            color = OhdColors.Red,
        )
        Spacer(Modifier.weight(1f))
        Icon(
            imageVector = OhdIcons.Sparkles,
            contentDescription = "CORD",
            tint = OhdColors.Muted,
            modifier = Modifier
                .size(22.dp)
                .clickable { onOpenCord() },
        )
        Icon(
            imageVector = OhdIcons.Bell,
            contentDescription = "Notifications",
            tint = OhdColors.Muted,
            modifier = Modifier
                .size(22.dp)
                .clickable { onOpenNotifications() },
        )
        Icon(
            imageVector = OhdIcons.Settings,
            contentDescription = "Settings",
            tint = OhdColors.Muted,
            modifier = Modifier
                .size(22.dp)
                .clickable { onOpenSettings() },
        )
    }
}

/** Start-of-range timestamp (ms) for [range] in the user's local timezone. */
private fun rangeStartMs(range: TimeRange): Long {
    val cal = Calendar.getInstance()
    when (range) {
        TimeRange.Today -> {
            cal.set(Calendar.HOUR_OF_DAY, 0)
            cal.set(Calendar.MINUTE, 0)
            cal.set(Calendar.SECOND, 0)
            cal.set(Calendar.MILLISECOND, 0)
        }
        TimeRange.Week -> cal.add(Calendar.DAY_OF_YEAR, -7)
        TimeRange.Month -> cal.add(Calendar.DAY_OF_YEAR, -30)
        TimeRange.Year -> cal.add(Calendar.DAY_OF_YEAR, -365)
    }
    return cal.timeInMillis
}

/** Human label for the stat-tile suffix. */
private fun rangeLabel(range: TimeRange): String = when (range) {
    TimeRange.Today -> "today"
    TimeRange.Week -> "this week"
    TimeRange.Month -> "this month"
    TimeRange.Year -> "this year"
}

/** Pure SQL COUNT(*) — no 10 000 row cap, fast even on year-range queries. */
private fun countEventsSince(fromMs: Long): Long =
    StorageRepository.countEvents(EventFilter(fromMs = fromMs, visibility = EventVisibility.TopLevelOnly))
        .getOrNull()
        ?: 0L

/** Compact thousands separator for the stat-tile big number. */
private fun formatCount(n: Long): String = when {
    n < 1000 -> n.toString()
    else -> "%,d".format(n)
}

/** "FAVOURITES" label + "+ Add" link (Pencil `T3h55`). */
@Composable
private fun FavouritesHeader(onAddFavourite: () -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = "FAVOURITES",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W600,
            fontSize = 11.sp,
            letterSpacing = 1.5.sp,
            color = OhdColors.Muted,
        )
        Spacer(Modifier.weight(1f))
        Text(
            text = "+ Add",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            color = OhdColors.Red,
            modifier = Modifier.clickable { onAddFavourite() },
        )
    }
}

/**
 * Favourites chip — Pencil §4.1 / row `S97si`.
 *
 * 28 dp tall, corner radius 20 dp, fill `ohd-bg-elevated`, 1 dp `ohd-line`
 * border, padding `[v=8, h=12]`, gap 6 between 16 dp Lucide icon (`ohd-red`)
 * and label (`Inter 13 / normal / ohd-ink`).
 */
@Composable
private fun FavouriteChip(
    label: String,
    icon: ImageVector,
    onClick: () -> Unit,
) {
    val shape = RoundedCornerShape(20.dp)
    Row(
        modifier = Modifier
            .height(36.dp)
            .background(OhdColors.BgElevated, shape)
            .border(BorderStroke(1.dp, OhdColors.Line), shape)
            .clickable { onClick() }
            .padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Icon(
            imageVector = icon,
            contentDescription = null,
            tint = OhdColors.Red,
            modifier = Modifier.size(16.dp),
        )
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Ink,
        )
    }
}
