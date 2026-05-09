package com.ohd.emergency.ui.components

import androidx.compose.foundation.background
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

import com.ohd.emergency.data.CaseVault
import com.ohd.emergency.ui.theme.EmergencyPalette

/**
 * Sync state surfaced in the top bar.
 *
 * Three signals fuse into one chip:
 *   - online / offline (transport reachability)
 *   - queue depth (how many writes are waiting)
 *   - flush in progress (a worker is currently posting)
 *
 * UX choice: a single chip with three colours rather than separate
 * indicators. Paramedics under stress shouldn't have to parse a row
 * of dots — one chip with a clear label is faster to scan.
 */
data class SyncIndicatorState(
    val status: CaseVault.SyncStatus,
    val queuedCount: Int,
)

@Composable
fun SyncIndicator(state: SyncIndicatorState) {
    val (label, dot) = when (state.status) {
        CaseVault.SyncStatus.Synced ->
            "Synced" to EmergencyPalette.Success

        CaseVault.SyncStatus.Queued ->
            "Queued (${state.queuedCount})" to EmergencyPalette.Warning

        CaseVault.SyncStatus.Syncing ->
            "Syncing…" to EmergencyPalette.Info

        CaseVault.SyncStatus.OfflineNoQueue ->
            "Offline" to EmergencyPalette.MutedDarker
    }

    val pillColor: Color = MaterialTheme.colorScheme.surfaceVariant
    val onPill: Color = MaterialTheme.colorScheme.onSurface

    Row(
        modifier = Modifier
            .clip(RoundedCornerShape(50))
            .background(pillColor)
            .padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(
            modifier = Modifier
                .size(10.dp)
                .clip(CircleShape)
                .background(dot)
        )
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = onPill,
        )
    }
}
