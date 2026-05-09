package com.ohd.emergency.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Backspace
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

import com.ohd.emergency.ui.theme.BigNumberStyle

/**
 * Big-button number pad, glove-friendly.
 *
 * Used by the intervention screen for HR / BP / SpO2 / temp / GCS
 * entry. Soft keyboard is **deliberately not used** per the brief:
 *
 *     Each is a card with chunky number-pad input (NOT a soft keyboard)
 *     for vitals.
 *
 * Why: the system soft keyboard is small, easy to mistype, and steals
 * vertical space in landscape. A 3×4 grid of dp-sized buttons is faster
 * for two-digit BP entry and works through paramedic gloves.
 *
 * Layout (3 columns × 4 rows):
 *
 *     | 1 | 2 | 3 |
 *     | 4 | 5 | 6 |
 *     | 7 | 8 | 9 |
 *     | . | 0 | ⌫ |
 *
 * The "." is shown but disabled for integer-only fields (HR, GCS); the
 * caller decides via [allowDecimal].
 */
@Composable
fun VitalsNumberPad(
    currentValue: String,
    label: String,
    unit: String,
    onAppend: (Char) -> Unit,
    onBackspace: () -> Unit,
    allowDecimal: Boolean = false,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier.fillMaxWidth()) {
        // Read-out at the top.
        Surface(
            modifier = Modifier
                .fillMaxWidth()
                .height(96.dp),
            shape = RoundedCornerShape(12.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHighest,
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Column {
                    Text(
                        text = label,
                        style = MaterialTheme.typography.labelLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Text(
                        text = currentValue.ifEmpty { "—" },
                        style = BigNumberStyle,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                }
                Text(
                    text = unit,
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }

        Spacer(Modifier.height(12.dp))

        // 3×4 grid of pad buttons.
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            PadRow(listOf("1", "2", "3"), onAppend)
            PadRow(listOf("4", "5", "6"), onAppend)
            PadRow(listOf("7", "8", "9"), onAppend)

            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                PadButton(
                    label = ".",
                    enabled = allowDecimal,
                    onClick = { onAppend('.') },
                    modifier = Modifier.weight(1f),
                )
                PadButton(
                    label = "0",
                    onClick = { onAppend('0') },
                    modifier = Modifier.weight(1f),
                )
                BackspaceButton(
                    onClick = onBackspace,
                    modifier = Modifier.weight(1f),
                )
            }
        }
    }
}

@Composable
private fun PadRow(digits: List<String>, onAppend: (Char) -> Unit) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        digits.forEach { d ->
            PadButton(
                label = d,
                onClick = { onAppend(d.first()) },
                modifier = Modifier.weight(1f),
            )
        }
    }
}

@Composable
private fun PadButton(
    label: String,
    enabled: Boolean = true,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Button(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier
            .aspectRatio(1.6f)
            .clip(RoundedCornerShape(12.dp)),
        shape = RoundedCornerShape(12.dp),
        colors = ButtonDefaults.buttonColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
            contentColor = MaterialTheme.colorScheme.onSurface,
            disabledContainerColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.3f),
        ),
    ) {
        Text(
            text = label,
            style = BigNumberStyle.copy(fontSize = 32.sp),
        )
    }
}

@Composable
private fun BackspaceButton(onClick: () -> Unit, modifier: Modifier = Modifier) {
    Button(
        onClick = onClick,
        modifier = modifier
            .aspectRatio(1.6f)
            .clip(RoundedCornerShape(12.dp)),
        shape = RoundedCornerShape(12.dp),
        colors = ButtonDefaults.buttonColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
            contentColor = MaterialTheme.colorScheme.onSurface,
        ),
    ) {
        Box(modifier = Modifier.padding(2.dp)) {
            Icon(imageVector = Icons.Filled.Backspace, contentDescription = "Backspace")
        }
    }
}

/** Helper for callers: append a digit/dot to a numeric string with sensible defaults. */
fun appendDigit(current: String, c: Char, maxLen: Int = 6): String {
    if (current.length >= maxLen) return current
    if (c == '.' && current.contains('.')) return current
    if (c == '.' && current.isEmpty()) return "0."
    return current + c
}

/** Helper for callers: drop the last char (or no-op). */
fun backspaceDigit(current: String): String =
    if (current.isEmpty()) current else current.dropLast(1)
