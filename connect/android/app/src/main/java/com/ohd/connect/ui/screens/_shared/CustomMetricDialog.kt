package com.ohd.connect.ui.screens._shared

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdField
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Value type the user picks for a new custom measurement. Mirrors the
 * subset of storage-core value types we surface to end users — the system
 * supports `enum` too but it's not exposed in v1 (no way to enter the
 * allowed values from this dialog).
 */
enum class CustomMetricValueType(val label: String, val storageKey: String) {
    Real("Real (decimal)", "real"),
    Int("Integer", "int"),
    Text("Text", "text"),
}

/**
 * One row in `Auth.custom_metrics_v1`. Carried through the dialog →
 * settings-screen → JSON pipeline.
 *
 * `namespace` is always `"custom"` for now (the runtime registry doesn't
 * accept other prefixes); leaving the field on the data class so a future
 * "promote to known" flow can reassign it without breaking persistence.
 */
data class CustomMetric(
    val namespace: String,
    val name: String,
    val description: String,
    val valueType: CustomMetricValueType,
    val unit: String?,
)

/**
 * Material3 dialog for the "+ New measurement" flow under Settings → Forms
 * & Measurements → Custom measurements.
 *
 * Fields:
 *  - Description (text, required) — user-facing label; we also derive a
 *    snake-case `name` from it (`"Ankle swelling"` → `"ankle_swelling"`).
 *  - Value type (radio: Real / Integer / Text).
 *  - Unit (text, optional) — free-form so the user can type `cm`, `bpm`,
 *    whatever.
 *
 * Persisting is the caller's responsibility — on Save we surface the
 * [CustomMetric] via [onSave]. The settings screen then appends it to
 * `custom_metrics_v1` via the [Auth] accessors.
 */
@Composable
fun CustomMetricDialog(
    onDismiss: () -> Unit,
    onSave: (CustomMetric) -> Unit,
) {
    var description by remember { mutableStateOf("") }
    var valueType by remember { mutableStateOf(CustomMetricValueType.Real) }
    var unit by remember { mutableStateOf("") }

    val canSave = description.trim().isNotEmpty()

    AlertDialog(
        onDismissRequest = onDismiss,
        title = {
            Text(
                text = "New custom measurement",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 16.sp,
                color = OhdColors.Ink,
            )
        },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                OhdField(
                    label = "Description",
                    value = description,
                    onValueChange = { description = it.take(60) },
                    placeholder = "e.g. Ankle swelling",
                )

                Text(
                    text = "Value type",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 13.sp,
                    color = OhdColors.Ink,
                )
                Column {
                    CustomMetricValueType.values().forEach { opt ->
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .selectable(
                                    selected = opt == valueType,
                                    onClick = { valueType = opt },
                                )
                                .padding(vertical = 4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(selected = opt == valueType, onClick = { valueType = opt })
                            Spacer(Modifier.height(0.dp))
                            Text(
                                text = opt.label,
                                fontFamily = OhdBody,
                                fontWeight = FontWeight.W400,
                                fontSize = 13.sp,
                                color = OhdColors.Ink,
                            )
                        }
                    }
                }

                OhdField(
                    label = "Unit (optional)",
                    value = unit,
                    onValueChange = { unit = it.take(20) },
                    placeholder = "cm, bpm, …",
                )
            }
        },
        confirmButton = {
            OhdButton(
                label = "Save",
                onClick = {
                    val desc = description.trim()
                    onSave(
                        CustomMetric(
                            namespace = "custom",
                            name = slugify(desc),
                            description = desc,
                            valueType = valueType,
                            unit = unit.trim().takeIf { it.isNotEmpty() },
                        )
                    )
                },
                variant = OhdButtonVariant.Primary,
                enabled = canSave,
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

/**
 * Convert a free-text description into a snake-case identifier. Keeps
 * digits and ASCII letters, collapses everything else to underscores, and
 * lowercases the result. We don't try to be locale-aware — the
 * `custom_metrics_v1` blob is per-install and the canonical registry
 * doesn't see these names anyway.
 */
private fun slugify(text: String): String {
    val out = StringBuilder()
    var lastWasUnderscore = true
    for (ch in text) {
        val c = when {
            ch in 'a'..'z' || ch in '0'..'9' -> ch
            ch in 'A'..'Z' -> ch + ('a' - 'A')
            else -> '_'
        }
        if (c == '_') {
            if (!lastWasUnderscore) out.append('_')
            lastWasUnderscore = true
        } else {
            out.append(c)
            lastWasUnderscore = false
        }
    }
    return out.toString().trim('_').ifEmpty { "metric" }
}
