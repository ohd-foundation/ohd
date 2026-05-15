package com.ohd.connect.ui.screens._shared

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Quick-measure sheet kinds — one entry per row in `MeasurementScreen`'s
 * QUICK MEASURES section. Mirrors the four spec rows from §4.9:
 * Blood pressure, Glucose, Body weight, Body temperature.
 */
enum class QuickMeasureKind { BloodPressure, Glucose, BodyWeight, BodyTemperature }

/**
 * Successful submission payload from [MeasurementEntrySheet]. Each variant
 * carries the user-typed numeric values plus, where relevant, the unit
 * picked via the chip toggle. The caller (today: `MeasurementScreen`)
 * translates this into the appropriate `EventInput` channel set and
 * persists via `StorageRepository.putEvent`.
 *
 * Keeping persistence outside the sheet so:
 *  1. The sheet stays a pure UI component (testable without uniffi).
 *  2. The success snackbar lives at the screen-level scaffold and can
 *     read the just-entered value to render a friendly message.
 */
sealed interface MeasurementEntry {
    /** mmHg, both fields are required positive integers. */
    data class BloodPressure(val systolic: Int, val diastolic: Int) : MeasurementEntry

    /** Real value + the unit picked in the toggle. */
    data class Glucose(val value: Double, val unit: GlucoseUnit) : MeasurementEntry

    data class BodyWeight(val value: Double, val unit: WeightUnit) : MeasurementEntry

    data class BodyTemperature(val value: Double, val unit: TemperatureUnit) : MeasurementEntry
}

enum class GlucoseUnit(val label: String) { MmolPerL("mmol/L"), MgPerDl("mg/dL") }
enum class WeightUnit(val label: String) { Kg("kg"), Lb("lb") }
enum class TemperatureUnit(val label: String) { Celsius("°C"), Fahrenheit("°F") }

/**
 * Modal bottom-sheet entry form for the four quick-measure kinds.
 *
 * Renders one of four bodies depending on [kind]:
 *
 *  - Blood pressure: two side-by-side numeric fields (systolic + diastolic),
 *    auto-focuses systolic.
 *  - Glucose: one numeric field + chip toggle `mmol/L` / `mg/dL`.
 *  - Body weight: one numeric field + chip toggle `kg` / `lb`.
 *  - Body temperature: one numeric field + chip toggle `°C` / `°F`.
 *
 * Bottom row: Cancel ghost + Log primary. The Log button only enables
 * when the parsed input is valid (positive number(s)).
 *
 * The composable does **not** itself persist — it surfaces a
 * [MeasurementEntry] via [onLog] and the caller owns the StorageRepository
 * call. This keeps the sheet trivially testable and lets the snackbar
 * message live with the screen scaffold.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MeasurementEntrySheet(
    kind: QuickMeasureKind,
    onDismiss: () -> Unit,
    onLog: (MeasurementEntry) -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        containerColor = OhdColors.Bg,
    ) {
        when (kind) {
            QuickMeasureKind.BloodPressure -> BloodPressureBody(onCancel = onDismiss, onLog = onLog)
            QuickMeasureKind.Glucose -> GlucoseBody(onCancel = onDismiss, onLog = onLog)
            QuickMeasureKind.BodyWeight -> BodyWeightBody(onCancel = onDismiss, onLog = onLog)
            QuickMeasureKind.BodyTemperature -> BodyTemperatureBody(onCancel = onDismiss, onLog = onLog)
        }
    }
}

// =============================================================================
// Per-kind sheet bodies
// =============================================================================

@Composable
private fun BloodPressureBody(
    onCancel: () -> Unit,
    onLog: (MeasurementEntry) -> Unit,
) {
    var systolic by remember { mutableStateOf("") }
    var diastolic by remember { mutableStateOf("") }
    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(Unit) { focusRequester.requestFocus() }

    val systolicInt = systolic.trim().toIntOrNull()
    val diastolicInt = diastolic.trim().toIntOrNull()
    val canLog = systolicInt != null && diastolicInt != null &&
        systolicInt > 0 && diastolicInt > 0

    SheetScaffold(
        title = "Blood pressure",
        canLog = canLog,
        onCancel = onCancel,
        onLog = {
            onLog(MeasurementEntry.BloodPressure(systolicInt!!, diastolicInt!!))
        },
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Box(modifier = Modifier.weight(1f)) {
                OhdField(
                    label = "Systolic (mmHg)",
                    value = systolic,
                    onValueChange = { raw -> systolic = raw.filter { it.isDigit() }.take(3) },
                    placeholder = "120",
                    keyboardType = KeyboardType.Number,
                    modifier = Modifier.focusRequester(focusRequester),
                )
            }
            Box(modifier = Modifier.weight(1f)) {
                OhdField(
                    label = "Diastolic (mmHg)",
                    value = diastolic,
                    onValueChange = { raw -> diastolic = raw.filter { it.isDigit() }.take(3) },
                    placeholder = "80",
                    keyboardType = KeyboardType.Number,
                )
            }
        }
    }
}

@Composable
private fun GlucoseBody(
    onCancel: () -> Unit,
    onLog: (MeasurementEntry) -> Unit,
) {
    var value by remember { mutableStateOf("") }
    var unit by remember { mutableStateOf(GlucoseUnit.MmolPerL) }
    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(Unit) { focusRequester.requestFocus() }

    val parsed = value.trim().toDoubleOrNull()
    val canLog = parsed != null && parsed > 0.0

    SheetScaffold(
        title = "Glucose",
        canLog = canLog,
        onCancel = onCancel,
        onLog = { onLog(MeasurementEntry.Glucose(parsed!!, unit)) },
    ) {
        OhdField(
            label = "Value (${unit.label})",
            value = value,
            onValueChange = { raw -> value = sanitiseDecimal(raw) },
            placeholder = if (unit == GlucoseUnit.MmolPerL) "5.4" else "97",
            keyboardType = KeyboardType.Decimal,
            modifier = Modifier.focusRequester(focusRequester),
        )
        Spacer(Modifier.height(12.dp))
        UnitChipRow(
            options = GlucoseUnit.values().toList(),
            selected = unit,
            labelOf = { it.label },
            onSelect = { unit = it },
        )
    }
}

@Composable
private fun BodyWeightBody(
    onCancel: () -> Unit,
    onLog: (MeasurementEntry) -> Unit,
) {
    var value by remember { mutableStateOf("") }
    var unit by remember { mutableStateOf(WeightUnit.Kg) }
    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(Unit) { focusRequester.requestFocus() }

    val parsed = value.trim().toDoubleOrNull()
    val canLog = parsed != null && parsed > 0.0

    SheetScaffold(
        title = "Body weight",
        canLog = canLog,
        onCancel = onCancel,
        onLog = { onLog(MeasurementEntry.BodyWeight(parsed!!, unit)) },
    ) {
        OhdField(
            label = "Value (${unit.label})",
            value = value,
            onValueChange = { raw -> value = sanitiseDecimal(raw) },
            placeholder = if (unit == WeightUnit.Kg) "72.5" else "160",
            keyboardType = KeyboardType.Decimal,
            modifier = Modifier.focusRequester(focusRequester),
        )
        Spacer(Modifier.height(12.dp))
        UnitChipRow(
            options = WeightUnit.values().toList(),
            selected = unit,
            labelOf = { it.label },
            onSelect = { unit = it },
        )
    }
}

@Composable
private fun BodyTemperatureBody(
    onCancel: () -> Unit,
    onLog: (MeasurementEntry) -> Unit,
) {
    var value by remember { mutableStateOf("") }
    var unit by remember { mutableStateOf(TemperatureUnit.Celsius) }
    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(Unit) { focusRequester.requestFocus() }

    val parsed = value.trim().toDoubleOrNull()
    val canLog = parsed != null && parsed > 0.0

    SheetScaffold(
        title = "Body temperature",
        canLog = canLog,
        onCancel = onCancel,
        onLog = { onLog(MeasurementEntry.BodyTemperature(parsed!!, unit)) },
    ) {
        OhdField(
            label = "Value (${unit.label})",
            value = value,
            onValueChange = { raw -> value = sanitiseDecimal(raw) },
            placeholder = if (unit == TemperatureUnit.Celsius) "36.7" else "98.0",
            keyboardType = KeyboardType.Decimal,
            modifier = Modifier.focusRequester(focusRequester),
        )
        Spacer(Modifier.height(12.dp))
        UnitChipRow(
            options = TemperatureUnit.values().toList(),
            selected = unit,
            labelOf = { it.label },
            onSelect = { unit = it },
        )
    }
}

// =============================================================================
// Shared scaffolding — title, body slot, Cancel + Log buttons.
// =============================================================================

@Composable
private fun SheetScaffold(
    title: String,
    canLog: Boolean,
    onCancel: () -> Unit,
    onLog: () -> Unit,
    body: @Composable () -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 20.dp, vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text(
            text = title,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W500,
            fontSize = 16.sp,
            color = OhdColors.Ink,
        )
        body()
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.spacedBy(10.dp, Alignment.End),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            OhdButton(
                label = "Cancel",
                onClick = onCancel,
                variant = OhdButtonVariant.Ghost,
            )
            OhdButton(
                label = "Log",
                onClick = onLog,
                variant = OhdButtonVariant.Primary,
                enabled = canLog,
            )
        }
        Spacer(Modifier.height(8.dp))
    }
}

/**
 * Generic unit-toggle chip row — two or more enum values rendered as
 * Pencil-style chips. Mirrors the pattern in `RetentionDialog` but
 * narrowed to the binary unit-pick case.
 */
@Composable
private fun <T> UnitChipRow(
    options: List<T>,
    selected: T,
    labelOf: (T) -> String,
    onSelect: (T) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        options.forEach { opt ->
            val isSelected = opt == selected
            Box(
                modifier = Modifier
                    .background(
                        if (isSelected) OhdColors.BgElevated else OhdColors.Bg,
                        RoundedCornerShape(4.dp),
                    )
                    .border(
                        BorderStroke(
                            if (isSelected) 1.5.dp else 1.dp,
                            if (isSelected) OhdColors.Ink else OhdColors.Line,
                        ),
                        RoundedCornerShape(4.dp),
                    )
                    .clickable { onSelect(opt) }
                    .padding(horizontal = 12.dp, vertical = 6.dp),
            ) {
                Text(
                    text = labelOf(opt),
                    fontFamily = OhdBody,
                    fontWeight = if (isSelected) FontWeight.W500 else FontWeight.W400,
                    fontSize = 13.sp,
                    color = OhdColors.Ink,
                )
            }
        }
    }
}

/**
 * Filter a raw text-field input to a sane decimal: digits and at most one
 * dot. We don't enforce magnitude — the call sites validate via
 * `toDoubleOrNull`.
 */
private fun sanitiseDecimal(raw: String): String {
    val filtered = raw.filter { it.isDigit() || it == '.' }
    val firstDot = filtered.indexOf('.')
    return if (firstDot < 0) {
        filtered.take(6)
    } else {
        // Keep up to one dot and at most 2 fractional digits.
        val intPart = filtered.substring(0, firstDot).take(4)
        val fracPart = filtered.substring(firstDot + 1).filter { it != '.' }.take(2)
        if (fracPart.isEmpty()) "$intPart." else "$intPart.$fracPart"
    }
}
