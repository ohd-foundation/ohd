package com.ohd.connect.ui.screens._shared

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import com.ohd.connect.data.RetentionLimits
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdInput
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Retention limits dialog — opens from the on-device storage card's
 * "Unlimited ▾" chip in `OnDeviceExpandedPanel`. Lets the user pick:
 *
 *  - **Max age** of stored data, in years (or unlimited).
 *  - **Max file size** of the on-device DB, in GB (or unlimited).
 *
 * Both controls follow the same chip-row + custom-text-field pattern.
 * Picking a chip clears the custom field; typing a custom value
 * deselects all chips and selects "Custom".
 *
 * Apply persists via `Auth.saveRetentionLimits(...)` — handled by the
 * caller via [onApply]. Cancel discards the unsaved selection.
 */
@Composable
fun RetentionDialog(
    initial: RetentionLimits,
    onDismiss: () -> Unit,
    onApply: (RetentionLimits) -> Unit,
) {
    // Preset chip options. Ordered as in the spec.
    val ageChips = listOf<Pair<String, Int?>>(
        "Unlimited" to null,
        "1 year" to 1,
        "2 years" to 2,
        "5 years" to 5,
    )
    val sizeChips = listOf<Pair<String, Int?>>(
        "Unlimited" to null,
        "1 GB" to 1,
        "5 GB" to 5,
        "20 GB" to 20,
    )

    // Determine if the initial value matches a chip; if not, treat the
    // value as a custom entry pre-populating the text field.
    val initialAgeChipMatch: String? = if (initial.maxAgeYears == null) {
        "Unlimited"
    } else {
        ageChips.firstOrNull { it.second == initial.maxAgeYears }?.first
    }
    val initialAgeCustom = if (initial.maxAgeYears != null && initialAgeChipMatch == null) {
        initial.maxAgeYears.toString()
    } else {
        ""
    }
    val initialAgeSelection = initialAgeChipMatch ?: if (initialAgeCustom.isNotEmpty()) "Custom" else "Unlimited"

    val initialSizeChipMatch: String? = if (initial.maxSizeGb == null) {
        "Unlimited"
    } else {
        sizeChips.firstOrNull { it.second == initial.maxSizeGb }?.first
    }
    val initialSizeCustom = if (initial.maxSizeGb != null && initialSizeChipMatch == null) {
        initial.maxSizeGb.toString()
    } else {
        ""
    }
    val initialSizeSelection = initialSizeChipMatch ?: if (initialSizeCustom.isNotEmpty()) "Custom" else "Unlimited"

    var ageChip by remember { mutableStateOf<String?>(initialAgeSelection) }
    var ageCustom by remember { mutableStateOf(initialAgeCustom) }

    var sizeChip by remember { mutableStateOf<String?>(initialSizeSelection) }
    var sizeCustom by remember { mutableStateOf(initialSizeCustom) }

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = true),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .background(OhdColors.Bg, RoundedCornerShape(12.dp))
                .border(BorderStroke(1.dp, OhdColors.Line), RoundedCornerShape(12.dp))
                .padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            Text(
                text = "Retention limits",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 16.sp,
                color = OhdColors.Ink,
            )

            // ---- Max age ------------------------------------------------------
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    text = "Max age",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 13.sp,
                    color = OhdColors.Ink,
                )
                ChipRow(
                    options = ageChips.map { it.first },
                    selected = ageChip,
                    onSelect = { label ->
                        ageChip = label
                        ageCustom = ""
                    },
                )
                CustomValueRow(
                    label = "Custom (years):",
                    value = ageCustom,
                    onValueChange = { newRaw ->
                        val newValue = newRaw.filter { it.isDigit() }.take(2)
                        ageCustom = newValue
                        if (newValue.isNotEmpty()) {
                            ageChip = "Custom"
                        } else if (ageChip == "Custom") {
                            ageChip = null
                        }
                    },
                )
            }

            // ---- Max file size ----------------------------------------------
            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Text(
                    text = "Max file size",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 13.sp,
                    color = OhdColors.Ink,
                )
                ChipRow(
                    options = sizeChips.map { it.first },
                    selected = sizeChip,
                    onSelect = { label ->
                        sizeChip = label
                        sizeCustom = ""
                    },
                )
                CustomValueRow(
                    label = "Custom (GB):",
                    value = sizeCustom,
                    onValueChange = { newRaw ->
                        val newValue = newRaw.filter { it.isDigit() }.take(3)
                        sizeCustom = newValue
                        if (newValue.isNotEmpty()) {
                            sizeChip = "Custom"
                        } else if (sizeChip == "Custom") {
                            sizeChip = null
                        }
                    },
                )
            }

            // ---- CTAs --------------------------------------------------------
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(10.dp, Alignment.End),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                OhdButton(
                    label = "Cancel",
                    onClick = onDismiss,
                    variant = OhdButtonVariant.Ghost,
                )
                OhdButton(
                    label = "Apply",
                    onClick = {
                        val maxAgeYears = resolveValue(ageChip, ageCustom, ageChips, min = 1, max = 50)
                        val maxSizeGb = resolveValue(sizeChip, sizeCustom, sizeChips, min = 1, max = 500)
                        onApply(RetentionLimits(maxAgeYears = maxAgeYears, maxSizeGb = maxSizeGb))
                    },
                    variant = OhdButtonVariant.Primary,
                )
            }
        }
    }
}

/**
 * Compute the integer year/GB value to persist from the (chip, custom)
 * pair. Returns `null` for "Unlimited" / unset, the chip value for a
 * named chip, or the parsed-and-clamped custom field for "Custom".
 */
private fun resolveValue(
    chip: String?,
    custom: String,
    chips: List<Pair<String, Int?>>,
    min: Int,
    max: Int,
): Int? {
    if (chip == "Custom") {
        val parsed = custom.toIntOrNull() ?: return null
        return parsed.coerceIn(min, max)
    }
    val match = chips.firstOrNull { it.first == chip } ?: return null
    return match.second
}

/**
 * Horizontal chip row used by both Max age and Max file size. The
 * currently-[selected] chip renders with an `ohd-ink` border and bold
 * label; everyone else uses the standard `ohd-line` chip style.
 */
@Composable
private fun ChipRow(
    options: List<String>,
    selected: String?,
    onSelect: (String) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        options.forEach { label ->
            val isSelected = selected == label
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
                    .clickable { onSelect(label) }
                    .padding(horizontal = 12.dp, vertical = 6.dp),
            ) {
                Text(
                    text = label,
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
 * "Custom (years):" / "Custom (GB):" inline row — small label on the
 * left, narrow `OhdInput` on the right.
 */
@Composable
private fun CustomValueRow(
    label: String,
    value: String,
    onValueChange: (String) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            color = OhdColors.Muted,
        )
        Box(modifier = Modifier.width(96.dp)) {
            OhdInput(
                value = value,
                onValueChange = onValueChange,
                placeholder = "",
                keyboardType = KeyboardType.Number,
            )
        }
    }
}

/**
 * Format a [RetentionLimits] pair for the "Unlimited ▾" chip in the
 * on-device card's expanded panel. Examples:
 *
 *  - both unlimited → "Unlimited"
 *  - age=2, size=null → "2 years"
 *  - age=null, size=5 → "5 GB"
 *  - age=2, size=5 → "5 GB · 2 years"
 */
fun formatRetentionLimits(limits: RetentionLimits): String {
    val agePart = limits.maxAgeYears?.let { years ->
        if (years == 1) "1 year" else "$years years"
    }
    val sizePart = limits.maxSizeGb?.let { gb -> "$gb GB" }
    return when {
        agePart == null && sizePart == null -> "Unlimited"
        agePart != null && sizePart != null -> "$sizePart · $agePart"
        sizePart != null -> sizePart
        else -> agePart!!
    }
}
