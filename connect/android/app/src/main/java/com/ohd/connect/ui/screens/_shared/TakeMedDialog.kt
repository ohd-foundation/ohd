package com.ohd.connect.ui.screens._shared

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
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
 * Long-press companion to the medication "Take" button.
 *
 * Lets the user override the time, dose amount and unit before persisting
 * the `medication.taken` event. The chip strips for time/unit follow the
 * same visual language as `OhdSegmentedTimeRange` (small ink-on-fill pill);
 * the custom-time field accepts `HH:MM` or `HH:MM:SS` and overrides the
 * selected chip when non-empty.
 *
 * Time chip semantics:
 *  - "Now"        → 0 ms offset
 *  - "5 min ago"  → -5 * 60 * 1000 ms
 *  - "30 min ago" → -30 * 60 * 1000 ms
 *  - "1 h ago"    → -60 * 60 * 1000 ms
 *
 * The custom field, if filled and parseable, overrides the chip and
 * resolves to today's `HH:MM[:SS]`. Unparseable input falls back to the
 * chip's offset (i.e. the dialog never silently mis-times the dose).
 */
@Composable
fun TakeMedDialog(
    medicationName: String,
    defaultDose: Double,
    defaultUnit: String,
    onDismiss: () -> Unit,
    onTake: (timestampMs: Long, dose: Double, unit: String) -> Unit,
) {
    var selectedOffset by remember { mutableStateOf(TimeOffset.Now) }
    var customTime by remember { mutableStateOf("") }
    var doseText by remember { mutableStateOf(formatDose(defaultDose)) }
    var unit by remember { mutableStateOf(defaultUnit) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Text(
                text = "Take $medicationName",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W600,
                fontSize = 16.sp,
                color = OhdColors.Ink,
            )
        },
        text = {
            Column(
                modifier = Modifier.fillMaxWidth(),
                verticalArrangement = Arrangement.spacedBy(14.dp),
            ) {
                // -- When --
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    SectionLabel("When")
                    ChipRow(
                        items = TimeOffset.values().toList(),
                        labelOf = { it.label },
                        isSelected = { it == selectedOffset && customTime.isBlank() },
                        onSelect = { selectedOffset = it },
                    )
                    OhdField(
                        label = "Custom time",
                        value = customTime,
                        onValueChange = { customTime = it },
                        placeholder = "HH:MM",
                    )
                }

                // -- Dose --
                OhdField(
                    label = "Dose",
                    value = doseText,
                    onValueChange = { doseText = it },
                    placeholder = "500",
                    keyboardType = KeyboardType.Decimal,
                )

                // -- Unit --
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    SectionLabel("Unit")
                    ChipRow(
                        items = COMMON_UNITS,
                        labelOf = { it },
                        isSelected = { it == unit },
                        onSelect = { unit = it },
                    )
                }
            }
        },
        confirmButton = {
            OhdButton(
                label = "Take",
                onClick = {
                    val ts = resolveTimestamp(selectedOffset, customTime)
                    val dose = doseText.replace(',', '.').toDoubleOrNull() ?: defaultDose
                    onTake(ts, dose, unit)
                },
                variant = OhdButtonVariant.Primary,
            )
        },
        dismissButton = {
            OhdButton(
                label = "Cancel",
                onClick = onDismiss,
                variant = OhdButtonVariant.Ghost,
            )
        },
        containerColor = OhdColors.Bg,
    )
}

@Composable
private fun SectionLabel(text: String) {
    Text(
        text = text,
        fontFamily = OhdBody,
        fontWeight = FontWeight.W500,
        fontSize = 13.sp,
        color = OhdColors.Ink,
    )
}

/**
 * Horizontally-scrollable strip of pill-shaped chips. Selected chip uses
 * `ohd-ink` fill with white text; inactive chips use a 1 dp `ohd-line`
 * border with muted text. Matches the look of `OhdSegmentedTimeRange`'s
 * active state but laid out as free-flowing pills (so the unit strip can
 * grow without recomputing fixed weights).
 */
@Composable
private fun <T> ChipRow(
    items: List<T>,
    labelOf: (T) -> String,
    isSelected: (T) -> Boolean,
    onSelect: (T) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        items.forEach { item ->
            val active = isSelected(item)
            val shape = RoundedCornerShape(20.dp)
            val bg = if (active) OhdColors.Ink else OhdColors.Bg
            val labelColor = if (active) OhdColors.White else OhdColors.Muted

            val baseModifier = Modifier
                .height(28.dp)
                .background(bg, shape)
                .clickable { onSelect(item) }
                .padding(horizontal = 10.dp)

            val finalModifier = if (active) baseModifier else baseModifier.border(
                width = 1.dp,
                color = OhdColors.Line,
                shape = shape,
            )

            Box(modifier = finalModifier, contentAlignment = Alignment.Center) {
                Text(
                    text = labelOf(item),
                    fontFamily = OhdBody,
                    fontWeight = if (active) FontWeight.W500 else FontWeight.W400,
                    fontSize = 12.sp,
                    color = labelColor,
                )
            }
        }
    }
}

/** Predefined time offsets for the chip strip. */
private enum class TimeOffset(val label: String, val offsetMs: Long) {
    Now("Now", 0L),
    FiveMin("5 min ago", -5L * 60_000L),
    ThirtyMin("30 min ago", -30L * 60_000L),
    OneHour("1 h ago", -60L * 60_000L),
}

/** Common dose units shown in the dialog's unit chip strip. */
private val COMMON_UNITS: List<String> = listOf("mg", "mL", "IU", "drops", "tablets")

/**
 * Resolve the chosen time. The custom field, if it parses as `HH:MM` or
 * `HH:MM:SS`, wins over the chip selection. Otherwise we apply the chip's
 * offset to `now`. We use `java.util.Calendar` rather than `java.time` so
 * we don't bump the project's minSdk.
 */
private fun resolveTimestamp(offset: TimeOffset, customTime: String): Long {
    val now = System.currentTimeMillis()
    val parsed = parseHhMm(customTime)
    if (parsed != null) {
        val cal = java.util.Calendar.getInstance()
        cal.set(java.util.Calendar.HOUR_OF_DAY, parsed.first)
        cal.set(java.util.Calendar.MINUTE, parsed.second)
        cal.set(java.util.Calendar.SECOND, parsed.third)
        cal.set(java.util.Calendar.MILLISECOND, 0)
        // If the user picked a time later than now (e.g. typed an evening
        // time before midnight rolls over) treat it as "yesterday at HH:MM"
        // so the timestamp is plausibly in the past.
        if (cal.timeInMillis > now) {
            cal.add(java.util.Calendar.DAY_OF_YEAR, -1)
        }
        return cal.timeInMillis
    }
    return now + offset.offsetMs
}

/** Parse `HH:MM` or `HH:MM:SS`. Returns `(h, m, s)` or null. */
internal fun parseHhMm(s: String): Triple<Int, Int, Int>? {
    val trimmed = s.trim()
    if (trimmed.isEmpty()) return null
    val parts = trimmed.split(":")
    if (parts.size !in 2..3) return null
    val h = parts[0].toIntOrNull() ?: return null
    val m = parts[1].toIntOrNull() ?: return null
    val sec = if (parts.size == 3) parts[2].toIntOrNull() ?: return null else 0
    if (h !in 0..23 || m !in 0..59 || sec !in 0..59) return null
    return Triple(h, m, sec)
}

/** Strip trailing `.0` so 500.0 reads "500" not "500.0" in the dialog. */
private fun formatDose(d: Double): String =
    if (d == d.toLong().toDouble()) d.toLong().toString() else d.toString()
