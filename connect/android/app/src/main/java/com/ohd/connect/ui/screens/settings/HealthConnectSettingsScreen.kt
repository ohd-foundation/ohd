package com.ohd.connect.ui.screens.settings

import android.content.ActivityNotFoundException
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Intent
import android.net.Uri
import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.HEALTH_CONNECT_TYPES
import com.ohd.connect.data.HealthConnectPrefs
import com.ohd.connect.data.HealthConnectScheduler
import com.ohd.connect.data.OhdHealthConnect
import com.ohd.connect.ui.components.OhdToggle
import com.ohd.connect.data.SyncProgress
import com.ohd.connect.data.SyncResult
import com.ohd.connect.data.syncFromHealthConnect
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdCard
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Health Connect settings — replaces the v0 "Coming soon" stub.
 *
 * Five sections, each rendered as an [OhdCard]:
 *
 *  1. **Status**       — provider availability + install link when missing.
 *  2. **Permissions**  — "X of Y granted" + a "Grant access" launcher.
 *  3. **Last sync**    — relative-time display + on-demand "Sync now".
 *  4. **Per-type**     — read-only checklist of the 8 record types with the
 *                        last sync's per-type counts beside them.
 *  5. **Debug**        — Changes-API token state, change/deletion counts,
 *                        and error strings from the last sync.
 *
 * The screen handles the `NotInstalled` case gracefully: every action
 * that would touch the SDK is gated on a non-null `client(ctx)` (or on
 * the `Installed` enum value), so on emulators without Health Connect
 * the user just sees the install link + a permanently-disabled "Sync now"
 * button. No crashes.
 */
@Composable
fun HealthConnectSettingsScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()

    // Live state. The availability + granted-permissions reads are cheap;
    // we re-run them every time the screen is composed so a returning
    // user (e.g. after granting in the Health Connect app) sees the
    // freshest state on the first frame.
    var availability by remember { mutableStateOf(OhdHealthConnect.availability(ctx)) }
    var grantedCount by remember { mutableStateOf(0) }
    var lastSyncMs by remember { mutableStateOf(HealthConnectPrefs.lastSyncMs(ctx)) }
    var syncing by remember { mutableStateOf(false) }
    var autoSyncEnabled by remember { mutableStateOf(HealthConnectScheduler.isEnabled(ctx)) }
    var lastResult by remember { mutableStateOf<SyncResult?>(null) }
    var snackbar by remember { mutableStateOf<String?>(null) }
    // Live synced-event count, published by syncFromHealthConnect as it runs
    // (covers both the manual "Sync now" and a background periodic sync
    // observed while this screen is open).
    val syncProgress by SyncProgress.state.collectAsState()

    // Refresh granted-permissions count when the screen first renders +
    // whenever availability flips (e.g. user installs Health Connect and
    // returns to the app).
    LaunchedEffect(availability) {
        grantedCount = if (availability == OhdHealthConnect.Availability.Installed) {
            runCatching { OhdHealthConnect.grantedPermissions(ctx).size }.getOrDefault(0)
        } else {
            0
        }
    }

    // Permission launcher — the result is the granted Set, so we update
    // the count from that rather than re-querying the SDK.
    val permissionLauncher = rememberHealthConnectPermissionLauncher { granted ->
        grantedCount = granted.intersect(OhdHealthConnect.PermissionsRead).size
    }

    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Health Connect", onBack = onBack)

        Column(
            modifier = Modifier
                .fillMaxSize()
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            // -------------------- 1. Status --------------------
            OhdCard(title = "Status") {
                StatusRow(availability)
                if (availability == OhdHealthConnect.Availability.NotInstalled) {
                    OhdButton(
                        label = "Install Health Connect",
                        variant = OhdButtonVariant.Secondary,
                        modifier = Modifier.fillMaxWidth(),
                        onClick = { openPlayStore(ctx) },
                    )
                } else if (availability == OhdHealthConnect.Availability.NeedsUpdate) {
                    OhdButton(
                        label = "Update Health Connect",
                        variant = OhdButtonVariant.Secondary,
                        modifier = Modifier.fillMaxWidth(),
                        onClick = { openPlayStore(ctx) },
                    )
                }
            }

            // -------------------- 2. Permissions --------------------
            OhdCard(title = "Permissions") {
                val total = OhdHealthConnect.PermissionsRead.size
                Text(
                    text = "$grantedCount of $total permissions granted",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 13.sp,
                    color = OhdColors.Muted,
                )
                OhdButton(
                    label = if (grantedCount == 0) "Grant access" else "Manage permissions",
                    onClick = { permissionLauncher.launch(OhdHealthConnect.PermissionsRead) },
                    enabled = availability == OhdHealthConnect.Availability.Installed,
                    modifier = Modifier.fillMaxWidth(),
                )
            }

            // -------------------- 3. Last sync --------------------
            OhdCard(title = "Sync") {
                Text(
                    text = "Last sync: ${formatLastSync(lastSyncMs)}",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 13.sp,
                    color = OhdColors.Muted,
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    OhdButton(
                        label = if (syncing) "Syncing…" else "Sync now",
                        onClick = onClick@{
                            if (syncing) return@onClick
                            scope.launch {
                                syncing = true
                                // HC sync writes thousands of events through
                                // SQLCipher; doing that on Main froze the UI
                                // until completion. Off-thread restores frame
                                // responsiveness while sync runs.
                                val result = runCatching {
                                    withContext(Dispatchers.IO) { syncFromHealthConnect(ctx) }
                                }
                                syncing = false
                                if (result.isSuccess) {
                                    val r = result.getOrThrow()
                                    lastResult = r
                                    lastSyncMs = HealthConnectPrefs.lastSyncMs(ctx)
                                    snackbar = "Imported ${r.ingested} events from Health Connect"
                                } else {
                                    snackbar = "Sync failed: ${result.exceptionOrNull()?.message ?: "(unknown)"}"
                                }
                            }
                        },
                        enabled = availability == OhdHealthConnect.Availability.Installed && !syncing,
                        modifier = Modifier.weight(1f),
                    )
                    if (syncing || syncProgress.running) {
                        CircularProgressIndicator(
                            modifier = Modifier.size(20.dp),
                            color = OhdColors.Red,
                            strokeWidth = 2.dp,
                        )
                        Text(
                            text = "${syncProgress.synced} event" +
                                (if (syncProgress.synced == 1) "" else "s") + " synced",
                            fontFamily = OhdBody,
                            fontWeight = FontWeight.W500,
                            fontSize = 13.sp,
                            color = OhdColors.Muted,
                        )
                    }
                }
                if (snackbar != null) {
                    Text(
                        text = snackbar!!,
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 12.sp,
                        color = OhdColors.Ink,
                    )
                }

                // ---- Auto-sync toggle ---------------------------------
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Column(modifier = Modifier.weight(1f)) {
                        Text(
                            text = "Auto-sync every 15 minutes",
                            fontFamily = OhdBody,
                            fontWeight = FontWeight.W500,
                            fontSize = 14.sp,
                            color = OhdColors.Ink,
                        )
                        Text(
                            text = "WorkManager runs in the background; the Health Connect " +
                                "Changes API pulls every new record in insertion order, " +
                                "including backdated samples.",
                            fontFamily = OhdBody,
                            fontWeight = FontWeight.W400,
                            fontSize = 12.sp,
                            color = OhdColors.Muted,
                        )
                    }
                    OhdToggle(
                        checked = autoSyncEnabled,
                        onCheckedChange = { newValue ->
                            autoSyncEnabled = newValue
                            HealthConnectScheduler.setEnabled(ctx, newValue)
                        },
                    )
                }
            }

            // -------------------- 4. Per-type list --------------------
            OhdCard(title = "Record types") {
                Text(
                    text = "Each row shows the lifetime event count for that record type. " +
                        "Tap a row to filter Recent Events to just that type.",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
                // Read lifetime counts from storage every time the screen
                // recomposes (cheap — pure COUNT(*)). The map is also
                // refreshed after a sync via the `lastSyncMs` change.
                val typeCounts = remember(lastSyncMs, lastResult) {
                    HEALTH_CONNECT_TYPES.associate { (_, eventType) ->
                        eventType to (
                            com.ohd.connect.data.StorageRepository
                                .countEvents(
                                    com.ohd.connect.data.EventFilter(
                                        eventTypesIn = listOf(eventType),
                                    ),
                                )
                                .getOrNull()
                                ?: 0L
                        )
                    }
                }
                HEALTH_CONNECT_TYPES.forEach { (label, eventType) ->
                    val lifetime = typeCounts[eventType] ?: 0L
                    val sinceLastSync = lastResult?.readByType?.get(eventType)
                    TypeRow(
                        label = label,
                        lifetime = lifetime,
                        sinceLastSync = sinceLastSync,
                        onClick = {
                            snackbar = "Filtered History by $label — coming soon"
                        },
                    )
                }
            }

            // -------------------- 5. Debug --------------------
            //
            // Always shown once Health Connect is installed — it surfaces
            // the Changes-API token state so a tester can confirm
            // incremental sync is armed, plus per-run change/deletion
            // counts and any errors from the last sync.
            if (availability == OhdHealthConnect.Availability.Installed) {
                OhdCard(title = "Debug") {
                    val tokenArmed = remember(lastResult, lastSyncMs) {
                        HealthConnectPrefs.changesToken(ctx) != null
                    }
                    val debugText = remember(lastResult, tokenArmed) {
                        buildString {
                            appendLine(
                                "changes token: " +
                                    if (tokenArmed) "armed (incremental)" else "none (will backfill)",
                            )
                            val r = lastResult
                            if (r == null) {
                                append("no sync run this session")
                            } else {
                                appendLine("last run mode: ${r.mode}")
                                appendLine("ingested=${r.ingested}")
                                appendLine("changesProcessed=${r.changesProcessed}")
                                appendLine("deletions=${r.deletions}")
                                val nonZero = r.readByType.filterValues { it > 0 }
                                if (nonZero.isNotEmpty()) {
                                    appendLine("readByType:")
                                    nonZero.entries
                                        .sortedBy { it.key }
                                        .forEach { (k, v) -> appendLine("  $k=$v") }
                                }
                                if (r.errors.isNotEmpty()) {
                                    appendLine("errors (${r.errors.size}):")
                                    r.errors.forEach { appendLine("  $it") }
                                }
                            }
                        }.trimEnd()
                    }
                    SelectionContainer {
                        Text(
                            text = debugText,
                            fontFamily = OhdBody,
                            fontWeight = FontWeight.W400,
                            fontSize = 11.sp,
                            color = OhdColors.Muted,
                        )
                    }
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(top = 8.dp),
                        horizontalArrangement = Arrangement.spacedBy(8.dp, Alignment.End),
                    ) {
                        // Drops the changes token and re-runs a full
                        // historical backfill — recovers a wedged sync
                        // and re-arms the Changes API from scratch.
                        OhdButton(
                            label = "Sync from scratch",
                            variant = OhdButtonVariant.Secondary,
                            enabled = !syncing,
                            onClick = onScratch@{
                                if (syncing) return@onScratch
                                scope.launch {
                                    syncing = true
                                    val result = runCatching {
                                        withContext(Dispatchers.IO) {
                                            syncFromHealthConnect(ctx, forceHistorical = true)
                                        }
                                    }
                                    syncing = false
                                    if (result.isSuccess) {
                                        val r = result.getOrThrow()
                                        lastResult = r
                                        lastSyncMs = HealthConnectPrefs.lastSyncMs(ctx)
                                        snackbar = "Backfilled ${r.ingested} events from Health Connect"
                                    } else {
                                        snackbar = "Sync failed: " +
                                            (result.exceptionOrNull()?.message ?: "(unknown)")
                                    }
                                }
                            },
                        )
                        OhdButton(
                            label = "Copy",
                            variant = OhdButtonVariant.Secondary,
                            onClick = {
                                val cm = ctx.getSystemService(ClipboardManager::class.java)
                                cm?.setPrimaryClip(ClipData.newPlainText("OHD sync debug", debugText))
                                snackbar = "Copied"
                            },
                        )
                    }
                }
            }
        }
    }
}

/**
 * Status row — coloured dot + provider state label. Mirrors the
 * "Provisioned / Pending / Failed" pattern from the operator screens.
 */
@Composable
private fun StatusRow(state: OhdHealthConnect.Availability) {
    val (label, color) = when (state) {
        OhdHealthConnect.Availability.Installed -> "Installed" to OhdColors.Success
        OhdHealthConnect.Availability.NeedsUpdate -> "Needs update" to OhdColors.Warn
        OhdHealthConnect.Availability.NotInstalled -> "Not installed" to OhdColors.Muted
    }
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(
            modifier = Modifier
                .size(10.dp)
                .background(color, CircleShape),
        )
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 14.sp,
            color = OhdColors.Ink,
        )
    }
}

/**
 * One row in the per-record-type list.
 *
 * Clickable row with the record-type label on the left and an event-count
 * column on the right. The right side shows:
 *   - the lifetime count (events ever ingested for that type), always;
 *   - optionally a "+N" badge if the most recent sync ingested any new
 *     events of that type, so the user can see what just landed.
 */
@Composable
private fun TypeRow(
    label: String,
    lifetime: Long,
    sinceLastSync: Int?,
    onClick: () -> Unit,
) {
    val shape = RoundedCornerShape(6.dp)
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .border(1.dp, OhdColors.LineSoft, shape)
            .clickable { onClick() }
            .padding(horizontal = 12.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Ink,
            modifier = Modifier.weight(1f),
        )
        if (sinceLastSync != null && sinceLastSync > 0) {
            Text(
                text = "+$sinceLastSync",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 11.sp,
                color = OhdColors.Red,
            )
        }
        Text(
            text = if (lifetime == 0L) "—" else "%,d".format(lifetime),
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            color = OhdColors.Muted,
        )
    }
}

/**
 * Open the Play Store on the Health Connect package. Tries `market://`
 * first (in-Play-Store deep link), falls back to the web URL when no
 * Play Store app is installed (corp / GMS-less devices).
 */
private fun openPlayStore(ctx: android.content.Context) {
    val tryIntent = Intent(Intent.ACTION_VIEW, Uri.parse(OhdHealthConnect.PLAY_STORE_URI))
        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
    runCatching { ctx.startActivity(tryIntent) }
        .onFailure {
            if (it is ActivityNotFoundException) {
                ctx.startActivity(
                    Intent(Intent.ACTION_VIEW, Uri.parse(OhdHealthConnect.PLAY_STORE_WEB_URL))
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                )
            }
        }
}

/**
 * "Never" / "2 hours ago" / absolute date for the last-sync timestamp.
 */
private fun formatLastSync(ms: Long?): String {
    if (ms == null) return "Never"
    val now = System.currentTimeMillis()
    val diff = (now - ms).coerceAtLeast(0L)
    val mins = diff / 60_000L
    return when {
        mins < 1L -> "just now"
        mins < 60L -> "$mins min ago"
        mins < 24 * 60L -> "${mins / 60} h ago"
        else -> {
            val days = mins / (24 * 60L)
            "$days d ago"
        }
    }
}
