package com.ohd.connect.ui.screens

import androidx.compose.foundation.background
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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.EventChannelInput
import com.ohd.connect.data.EventInput
import com.ohd.connect.data.MetricDef
import com.ohd.connect.data.MetricsRegistry
import com.ohd.connect.data.OhdScalar
import com.ohd.connect.data.PutEventOutcome
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdInput
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.components.TopBarAction
import com.ohd.connect.ui.screens._shared.MeasurementEntry
import com.ohd.connect.ui.screens._shared.MeasurementEntrySheet
import com.ohd.connect.ui.screens._shared.QuickMeasureKind
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import org.json.JSONObject

/**
 * Measurement log — Pencil `tnEmm.png`, spec §4.9, extended for tracked
 * measurements (plan deep-dancing-teacup.md).
 *
 * Sections, top to bottom:
 *  - **ON A TREATMENT PLAN** — measurement *watches* tied to a case
 *    (e.g. "watch your temperature daily" ordered at a visit).
 *  - **TRACKED** — personal watches (no case) the user set up themselves.
 *  - **QUICK MEASURES** — the canonical one-off entry list (a reading you
 *    take once, e.g. a blood-pressure check in a mall) via
 *    `StorageRepository.putEvent`. This is the unchanged default path.
 *  - **CUSTOM FORMS** — urine strip / pain score dedicated screens.
 *
 * A watch only declares intent + schedule; readings are ordinary
 * `measurement.*` events — tapping a watch row opens the same entry sheet a
 * quick measure uses. Watches are read/written through the MCP tools
 * (`list_measurement_watches` / `start_measurement_watch` /
 * `stop_measurement_watch`), so a watch set up in chat shows up here.
 *
 * [preselectKind] is honoured once on first composition — favourites taps
 * from Home pass `?preselect=glucose` so the user lands on the entry sheet.
 */
@Composable
fun MeasurementScreen(
    onBack: () -> Unit,
    onLog: () -> Unit,
    onOpenUrineStrip: () -> Unit,
    onOpenPainScore: () -> Unit,
    onToast: (String) -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
    modifier: Modifier = Modifier,
    preselectKind: QuickMeasureKind? = null,
) {
    var openSheetFor by remember { mutableStateOf<QuickMeasureKind?>(null) }
    var watches by remember { mutableStateOf<List<Watch>>(emptyList()) }
    var refreshTick by remember { mutableStateOf(0) }
    var trackOpen by remember { mutableStateOf(false) }
    var stopTarget by remember { mutableStateOf<Watch?>(null) }

    // putEvent is a blocking network RPC against the remote backend — run it
    // off the main thread to avoid freezing the UI (ANR).
    val scope = rememberCoroutineScope()

    LaunchedEffect(Unit) {
        if (preselectKind != null) {
            openSheetFor = preselectKind
        }
    }

    LaunchedEffect(refreshTick) {
        val res = withContext(Dispatchers.IO) {
            StorageRepository.executeToolJson("list_measurement_watches", "{}")
        }
        res.getOrNull()?.let { raw ->
            runCatching {
                val arr = JSONObject(raw).optJSONArray("watches")
                watches = (0 until (arr?.length() ?: 0)).mapNotNull { i ->
                    val o = arr!!.optJSONObject(i) ?: return@mapNotNull null
                    val id = o.optString("watch_id", "").ifEmpty { return@mapNotNull null }
                    Watch(
                        watchId = id,
                        metric = o.optString("metric", ""),
                        label = o.optString("label", "").ifEmpty { null },
                        schedule = o.optString("schedule", "").ifEmpty { null },
                        caseId = o.optString("case_id", "").ifEmpty { null },
                    )
                }
            }
        }
    }

    fun startWatch(metric: String, schedule: String, quick: Boolean) {
        scope.launch(Dispatchers.IO) {
            val body = JSONObject().put("metric", metric).put("quick", quick)
            if (schedule.isNotBlank()) body.put("schedule", schedule)
            StorageRepository.executeToolJson("start_measurement_watch", body.toString())
            withContext(Dispatchers.Main) { onToast("Tracking ${metricLabel(metric)}") }
            refreshTick++
        }
    }

    fun stopWatch(w: Watch) {
        scope.launch(Dispatchers.IO) {
            StorageRepository.executeToolJson(
                "stop_measurement_watch",
                JSONObject().put("watch_id", w.watchId).toString(),
            )
            withContext(Dispatchers.Main) { onToast("Stopped tracking ${metricLabel(w.metric)}") }
            refreshTick++
        }
    }

    val planWatches = watches.filter { !it.caseId.isNullOrBlank() }
    val trackedWatches = watches.filter { it.caseId.isNullOrBlank() }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(
            title = "Measurement",
            onBack = onBack,
            action = TopBarAction(label = "Log", onClick = onLog),
        )

        val quickMeasures = remember { MetricsRegistry.quickMeasures() }

        LazyColumn(modifier = Modifier.fillMaxWidth()) {
            // ---- ON A TREATMENT PLAN (case-linked watches) ----
            if (planWatches.isNotEmpty()) {
                item { OhdSectionHeader(text = "ON A TREATMENT PLAN") }
                watchRows(planWatches, onLogReading = { w ->
                    metricToKind(w.metric)?.let { openSheetFor = it }
                }, onStop = { stopTarget = it })
                item { Spacer(Modifier.height(8.dp)) }
            }

            // ---- TRACKED (personal watches) ----
            if (trackedWatches.isNotEmpty()) {
                item { OhdSectionHeader(text = "TRACKED") }
                watchRows(trackedWatches, onLogReading = { w ->
                    metricToKind(w.metric)?.let { openSheetFor = it }
                }, onStop = { stopTarget = it })
                item { Spacer(Modifier.height(8.dp)) }
            }

            // ---- QUICK MEASURES (one-off entry) ----
            item { OhdSectionHeader(text = "QUICK MEASURES") }
            quickMeasures.forEachIndexed { idx, metric ->
                val kind = metric.toQuickMeasureKindOrNull()
                if (kind != null) {
                    item {
                        OhdListItem(
                            primary = metric.description.simplifyMeasurementLabel(),
                            secondary = metric.quickMeasureSecondary(),
                            meta = "›",
                            onClick = { openSheetFor = kind },
                        )
                    }
                    if (idx < quickMeasures.lastIndex) item { OhdDivider() }
                }
            }
            item { OhdDivider() }
            item {
                OhdListItem(
                    primary = "+ Track a measurement",
                    secondary = "Get it as a recurring item with a schedule",
                    meta = "›",
                    onClick = { trackOpen = true },
                )
            }

            // ---- CUSTOM FORMS ----
            item { Spacer(modifier = Modifier.height(8.dp)) }
            item { OhdSectionHeader(text = "CUSTOM FORMS") }
            item {
                OhdListItem(
                    primary = "Urine strip",
                    secondary = "8 fields · glucose, protein, pH…",
                    meta = "›",
                    onClick = onOpenUrineStrip,
                )
            }
            item { OhdDivider() }
            item {
                OhdListItem(
                    primary = "Pain score",
                    secondary = "2 fields · location, intensity",
                    meta = "›",
                    onClick = onOpenPainScore,
                )
            }
        }
    }

    val kind = openSheetFor
    if (kind != null) {
        MeasurementEntrySheet(
            kind = kind,
            onDismiss = { openSheetFor = null },
            onLog = { entry ->
                val input = entry.toEventInput()
                val summary = entry.toSnackbarSummary()
                scope.launch(Dispatchers.IO) {
                    val result = StorageRepository.putEvent(input).getOrElse { e ->
                        PutEventOutcome.Error(
                            code = "INTERNAL",
                            message = e.message ?: e::class.simpleName.orEmpty(),
                        )
                    }
                    withContext(Dispatchers.Main) {
                        when (result) {
                            is PutEventOutcome.Committed -> {
                                onToast("Logged $summary")
                                openSheetFor = null
                            }
                            is PutEventOutcome.Pending -> {
                                onToast("Pending review · $summary")
                                openSheetFor = null
                            }
                            is PutEventOutcome.Error -> {
                                onToast("Couldn't log: ${result.message}")
                            }
                        }
                    }
                }
            },
        )
    }

    if (trackOpen) {
        TrackMeasurementDialog(
            onDismiss = { trackOpen = false },
            onTrack = { metric, schedule, quick ->
                trackOpen = false
                startWatch(metric, schedule, quick)
            },
        )
    }

    stopTarget?.let { w ->
        AlertDialog(
            onDismissRequest = { stopTarget = null },
            title = { Text(w.label ?: metricLabel(w.metric)) },
            text = {
                Text(
                    "Stop tracking this measurement? Past readings are kept.",
                    fontFamily = OhdBody, fontSize = 13.sp, color = OhdColors.Muted,
                )
            },
            confirmButton = {
                TextButton(onClick = { stopWatch(w); stopTarget = null }) {
                    Text("Stop tracking", color = OhdColors.Red)
                }
            },
            dismissButton = {
                TextButton(onClick = { stopTarget = null }) { Text("Cancel") }
            },
        )
    }
}

/** An active measurement watch, parsed from list_measurement_watches. */
private data class Watch(
    val watchId: String,
    val metric: String,
    val label: String?,
    val schedule: String?,
    val caseId: String?,
)

/** Emit the rows for a list of watches into a LazyColumn. */
private fun androidx.compose.foundation.lazy.LazyListScope.watchRows(
    watches: List<Watch>,
    onLogReading: (Watch) -> Unit,
    onStop: (Watch) -> Unit,
) {
    watches.forEachIndexed { idx, w ->
        item {
            OhdListItem(
                primary = w.label ?: metricLabel(w.metric),
                secondary = scheduleLabel(w.schedule) ?: "tracked",
                meta = "Log ›",
                onClick = { onLogReading(w) },
                onLongClick = { onStop(w) },
            )
        }
        if (idx < watches.lastIndex) item { OhdDivider() }
    }
}

/** Friendly label for a watch's metric token. */
private fun metricLabel(metric: String): String = when (metric) {
    "blood_pressure" -> "Blood pressure"
    "glucose" -> "Glucose"
    "weight", "body_weight" -> "Body weight"
    "temperature", "body_temperature" -> "Body temperature"
    "heart_rate" -> "Heart rate"
    "spo2" -> "SpO₂"
    else -> metric.replace('_', ' ').replaceFirstChar { it.uppercase() }
}

/** Map a watch's metric token to the entry-sheet kind, when one exists. */
private fun metricToKind(metric: String): QuickMeasureKind? = when (metric) {
    "blood_pressure" -> QuickMeasureKind.BloodPressure
    "glucose" -> QuickMeasureKind.Glucose
    "weight", "body_weight" -> QuickMeasureKind.BodyWeight
    "temperature", "body_temperature" -> QuickMeasureKind.BodyTemperature
    else -> null
}

/** `anchor:<name>` → readable phrase; cron / free text shown as-is. */
private fun scheduleLabel(s: String?): String? {
    if (s.isNullOrBlank()) return null
    if (!s.startsWith("anchor:")) return s
    return when (val a = s.removePrefix("anchor:")) {
        "as_needed" -> "as needed"
        "waking" -> "on waking"
        "first_food" -> "with first food"
        "bedtime" -> "at bedtime"
        "each_meal" -> "with each meal"
        else -> "with $a"
    }
}

/**
 * Pick a metric + schedule to track regularly. The metric choices are the
 * quick-measure kinds that have an entry sheet (so a tracked row can be
 * logged in-place); the schedule is loose text (cron or anchor) for now.
 */
@Composable
private fun TrackMeasurementDialog(
    onDismiss: () -> Unit,
    onTrack: (metric: String, schedule: String, quick: Boolean) -> Unit,
) {
    var metric by remember { mutableStateOf("blood_pressure") }
    var schedule by remember { mutableStateOf("") }
    var quick by remember { mutableStateOf(true) }
    val choices = listOf("blood_pressure", "glucose", "weight", "temperature")
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Track a measurement") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Text("What to track", fontFamily = OhdBody, fontSize = 12.sp, color = OhdColors.Muted)
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    choices.forEach { m ->
                        OhdButton(
                            label = metricLabel(m),
                            variant = if (m == metric) OhdButtonVariant.Primary else OhdButtonVariant.Ghost,
                            onClick = { metric = m },
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                }
                OhdInput(
                    value = schedule, onValueChange = { schedule = it },
                    placeholder = "Schedule (e.g. daily, anchor:bedtime)",
                )
                Row(modifier = Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        "Show in my quick list",
                        fontFamily = OhdBody, fontSize = 14.sp, color = OhdColors.Ink,
                        modifier = Modifier.weight(1f),
                    )
                    Switch(checked = quick, onCheckedChange = { quick = it })
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onTrack(metric, schedule.trim(), quick) }) { Text("Track") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

// =============================================================================
// MeasurementEntry → EventInput / snackbar message
// =============================================================================

/**
 * Translate a [MeasurementEntry] into the [EventInput] shape expected by
 * `StorageRepository.putEvent`.
 *
 * Event-type and channel paths are flat strings (no `std.` prefix) so
 * each measurement type ships as a self-describing event. Channel paths:
 *
 *   - `measurement.blood_pressure` → `systolic_mmhg` (real) + `diastolic_mmhg` (real)
 *   - `measurement.glucose`        → `value` (real) + `unit` (text: "mmol/L" / "mg/dL")
 *   - `measurement.weight`         → `value` (real) + `unit` (text: "kg" / "lb")
 *   - `measurement.temperature`    → `value` (real) + `unit` (text: "°C" / "°F")
 *
 * Other agents wiring measurement-shaped events should mirror this shape.
 */
private fun MeasurementEntry.toEventInput(): EventInput {
    val now = System.currentTimeMillis()
    return when (this) {
        is MeasurementEntry.BloodPressure -> EventInput(
            timestampMs = now,
            eventType = "measurement.blood_pressure",
            channels = listOf(
                EventChannelInput(
                    path = "systolic_mmhg",
                    scalar = OhdScalar.Real(systolic.toDouble()),
                ),
                EventChannelInput(
                    path = "diastolic_mmhg",
                    scalar = OhdScalar.Real(diastolic.toDouble()),
                ),
            ),
        )
        is MeasurementEntry.Glucose -> EventInput(
            timestampMs = now,
            eventType = "measurement.glucose",
            channels = listOf(
                EventChannelInput(path = "value", scalar = OhdScalar.Real(value)),
                EventChannelInput(path = "unit", scalar = OhdScalar.Text(unit.label)),
            ),
        )
        is MeasurementEntry.BodyWeight -> EventInput(
            timestampMs = now,
            eventType = "measurement.weight",
            channels = listOf(
                EventChannelInput(path = "value", scalar = OhdScalar.Real(value)),
                EventChannelInput(path = "unit", scalar = OhdScalar.Text(unit.label)),
            ),
        )
        is MeasurementEntry.BodyTemperature -> EventInput(
            timestampMs = now,
            eventType = "measurement.temperature",
            channels = listOf(
                EventChannelInput(path = "value", scalar = OhdScalar.Real(value)),
                EventChannelInput(path = "unit", scalar = OhdScalar.Text(unit.label)),
            ),
        )
    }
}

/**
 * One-line summary used by the success snackbar. Mirrors the user-typed
 * value so they can tell at a glance which entry committed.
 */
private fun MeasurementEntry.toSnackbarSummary(): String = when (this) {
    is MeasurementEntry.BloodPressure -> "blood pressure $systolic/$diastolic mmHg"
    is MeasurementEntry.Glucose -> "glucose ${formatNum(value)} ${unit.label}"
    is MeasurementEntry.BodyWeight -> "weight ${formatNum(value)} ${unit.label}"
    is MeasurementEntry.BodyTemperature -> "temperature ${formatNum(value)} ${unit.label}"
}

/** Trim trailing `.0` so "5.4" reads as "5.4" but "5.0" reads as "5". */
private fun formatNum(v: Double): String {
    return if (v == v.toLong().toDouble()) v.toLong().toString() else v.toString()
}

// =============================================================================
// MetricsRegistry → MeasurementScreen helpers
// =============================================================================

/**
 * Map a registry [MetricDef] to the matching [QuickMeasureKind] so the row
 * tap opens the right [MeasurementEntrySheet] body.
 *
 * Returns null for `measurement.*` rows that don't have a sheet body yet
 * (heart_rate, spo2, urine_strip, pain) — those types come from Health
 * Connect or have dedicated screens, so they're filtered out of QUICK
 * MEASURES via `discoverable_in_quick_log = false` in the TOML.
 */
private fun MetricDef.toQuickMeasureKindOrNull(): QuickMeasureKind? =
    when (name) {
        "blood_pressure" -> QuickMeasureKind.BloodPressure
        "glucose" -> QuickMeasureKind.Glucose
        "weight" -> QuickMeasureKind.BodyWeight
        "temperature" -> QuickMeasureKind.BodyTemperature
        else -> null
    }

/**
 * Strip the "(Connect Android …)" suffix from the canonical description so
 * the screen shows a clean "Blood pressure" / "Body weight" label instead
 * of the storage-side debug-tag descriptor.
 */
private fun String.simplifyMeasurementLabel(): String =
    substringBefore(" (").trim()

/**
 * Render the muted secondary line under each quick-measure row, e.g.
 * "systolic / diastolic · mmHg" or "mmol/L or mg/dL".
 *
 * Falls back to listing channel paths when neither `unit_options` nor
 * uniform-unit channels are present.
 */
private fun MetricDef.quickMeasureSecondary(): String {
    val opts = unitOptions
    if (opts.isNotEmpty()) {
        return opts.joinToString(" or ")
    }
    // Same unit on every channel? Use "<name> / <name> · <unit>".
    val units = channels.mapNotNull { it.unit }.distinct()
    if (units.size == 1 && channels.size > 1) {
        return channels.joinToString(" / ") {
            it.name.removeSuffix("_${units[0].lowercase()}")
                .removeSuffix("_mmhg")
        } + " · " + units[0]
    }
    if (units.size == 1) return units[0]
    return channels.joinToString(" / ") { it.name }
}
