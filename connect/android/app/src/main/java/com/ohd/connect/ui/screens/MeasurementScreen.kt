package com.ohd.connect.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ohd.connect.data.EventChannelInput
import com.ohd.connect.data.EventInput
import com.ohd.connect.data.MetricDef
import com.ohd.connect.data.MetricsRegistry
import com.ohd.connect.data.OhdScalar
import com.ohd.connect.data.PutEventOutcome
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdSectionHeader
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.components.TopBarAction
import com.ohd.connect.ui.screens._shared.MeasurementEntry
import com.ohd.connect.ui.screens._shared.MeasurementEntrySheet
import com.ohd.connect.ui.screens._shared.QuickMeasureKind
import com.ohd.connect.ui.theme.OhdColors

/**
 * Measurement log — Pencil `tnEmm.png`, spec §4.9.
 *
 * Two sections (Quick measures + Custom forms) of [OhdListItem]s with `›`
 * meta. Quick measures open an inline [MeasurementEntrySheet] for value
 * entry; on submit the screen persists via `StorageRepository.putEvent`
 * and surfaces a snackbar via [onToast].
 *
 * Custom-form rows route to dedicated screens (`Urine strip` →
 * [UrineStripScreen]).
 *
 * The optional [preselectKind] is honoured on first composition only —
 * favourites taps from Home pass `?preselect=glucose` (etc.) so the user
 * lands directly on the entry sheet without having to tap the row.
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

    // Honour the preselect arg exactly once. We key the LaunchedEffect on
    // `Unit` so re-entries from the back stack don't keep re-popping the
    // sheet — the user explicitly closes it via Cancel/Log.
    LaunchedEffect(Unit) {
        if (preselectKind != null) {
            openSheetFor = preselectKind
        }
    }

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

        // Quick measures come from the canonical registry — see
        // `spec/registry/metrics.toml`, regenerated into `MetricsRegistry.kt`.
        // The four rows here are `measurement.*` event types flagged
        // `discoverable_in_quick_log = true`. Order matches TOML declaration.
        val quickMeasures = remember { MetricsRegistry.quickMeasures() }

        LazyColumn(modifier = Modifier.fillMaxWidth()) {
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

            // Custom forms — extra top padding per spec (`[t=16, b=8, h=16]`
            // on the section header itself; we add an 8 dp spacer above).
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
                val result = StorageRepository.putEvent(input).getOrElse { e ->
                    PutEventOutcome.Error(
                        code = "INTERNAL",
                        message = e.message ?: e::class.simpleName.orEmpty(),
                    )
                }
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
                        // Keep the sheet open so the user can retry without
                        // re-typing the value.
                        onToast("Couldn't log: ${result.message}")
                    }
                }
            },
        )
    }
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
