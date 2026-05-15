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

/** Common dose units rendered as a chip strip in the add-on-hand dialog. */
private val ON_HAND_UNITS: List<String> = listOf("mg", "mL", "IU", "drops", "tablets")

/**
 * Dialog for adding a medication to the user's on-hand list.
 *
 * Shape:
 *  - Name (text)
 *  - Dose (number; decimal allowed)
 *  - Unit (chip strip)
 *
 * v1 doesn't yet persist the row to a "user medication list" — the caller
 * surfaces a `"Added X to on-hand"` snackbar and the dialog dismisses.
 * Wiring storage is gated on landing a real prescribed-meds persistence
 * layer (out of scope for this beta cut).
 */
@Composable
fun AddOnHandDialog(
    onDismiss: () -> Unit,
    onAdd: (name: String, dose: Double?, unit: String) -> Unit,
) {
    var name by remember { mutableStateOf("") }
    var doseText by remember { mutableStateOf("") }
    var unit by remember { mutableStateOf(ON_HAND_UNITS.first()) }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Text(
                text = "Add to on-hand",
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
                OhdField(
                    label = "Name",
                    value = name,
                    onValueChange = { name = it },
                    placeholder = "e.g. Vitamin D3",
                )
                OhdField(
                    label = "Dose",
                    value = doseText,
                    onValueChange = { doseText = it },
                    placeholder = "500",
                    keyboardType = KeyboardType.Decimal,
                )
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(
                        text = "Unit",
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W500,
                        fontSize = 13.sp,
                        color = OhdColors.Ink,
                    )
                    UnitChipStrip(
                        units = ON_HAND_UNITS,
                        selected = unit,
                        onSelect = { unit = it },
                    )
                }
            }
        },
        confirmButton = {
            OhdButton(
                label = "Add",
                onClick = {
                    val trimmed = name.trim()
                    if (trimmed.isNotEmpty()) {
                        val dose = doseText.replace(',', '.').toDoubleOrNull()
                        onAdd(trimmed, dose, unit)
                    }
                },
                variant = OhdButtonVariant.Primary,
                enabled = name.trim().isNotEmpty(),
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
private fun UnitChipStrip(
    units: List<String>,
    selected: String,
    onSelect: (String) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        units.forEach { u ->
            val active = u == selected
            val shape = RoundedCornerShape(20.dp)
            val bg = if (active) OhdColors.Ink else OhdColors.Bg
            val labelColor = if (active) OhdColors.White else OhdColors.Muted

            val base = Modifier
                .height(28.dp)
                .background(bg, shape)
                .clickable { onSelect(u) }
                .padding(horizontal = 10.dp)
            val finalMod = if (active) base else base.border(1.dp, OhdColors.Line, shape)

            Box(modifier = finalMod, contentAlignment = Alignment.Center) {
                Text(
                    text = u,
                    fontFamily = OhdBody,
                    fontWeight = if (active) FontWeight.W500 else FontWeight.W400,
                    fontSize = 12.sp,
                    color = labelColor,
                )
            }
        }
    }
}
