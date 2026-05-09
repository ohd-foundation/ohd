package com.ohd.emergency.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp

import com.ohd.emergency.ui.theme.EmergencyPalette

/**
 * Inline status chip.
 *
 * Three semantic colours per the brief — Red (urgent), Amber (auto-grant),
 * Info (neutral). [Tone] picks the colour set; the chip renders a small
 * dot + label pill.
 */
enum class ChipTone { Critical, AutoGrant, Success, Warning, Info, Neutral }

@Composable
fun StatusChip(
    label: String,
    tone: ChipTone = ChipTone.Neutral,
    modifier: Modifier = Modifier,
    outlined: Boolean = false,
) {
    val color = when (tone) {
        ChipTone.Critical -> EmergencyPalette.RedBright
        ChipTone.AutoGrant -> EmergencyPalette.AutoGrant
        ChipTone.Success -> EmergencyPalette.Success
        ChipTone.Warning -> EmergencyPalette.Warning
        ChipTone.Info -> EmergencyPalette.Info
        ChipTone.Neutral -> MaterialTheme.colorScheme.onSurfaceVariant
    }

    val containerColor = if (outlined) Color.Transparent else MaterialTheme.colorScheme.surfaceVariant
    val borderColor = if (outlined) color else Color.Transparent

    Row(
        modifier = modifier
            .clip(RoundedCornerShape(50))
            .background(containerColor)
            .border(width = 1.dp, color = borderColor, shape = RoundedCornerShape(50))
            .padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(
            modifier = Modifier
                .size(8.dp)
                .clip(CircleShape)
                .background(color)
        )
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurface,
        )
    }
}
