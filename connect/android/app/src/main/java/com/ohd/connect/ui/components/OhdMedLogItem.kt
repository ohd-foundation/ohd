@file:OptIn(ExperimentalFoundationApi::class)

package com.ohd.connect.ui.components

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/** Whether the prescribed/on-hand med has already been taken in the current window. */
enum class TakenState { Pending, Taken }

/**
 * Medication log row — Pencil `hAKak`.
 *
 * Horizontal, padding `[v=14, h=16]`, gap 12, alignItems center. Left text
 * block (name + sub), right "Log" / "Taken" affordance:
 *   - 60 × 32 frame, corner `radius-md`.
 *   - Pending → fill `ohd-red`, label "Log" `Inter 12 / 500 / #FFFFFF`.
 *   - Taken   → fill `ohd-bg` + 1 dp `ohd-line` border, label "Taken"
 *               `Inter 12 / 500 / ohd-muted`.
 */
@Composable
fun OhdMedLogItem(
    name: String,
    sub: String,
    modifier: Modifier = Modifier,
    takenState: TakenState = TakenState.Pending,
    onLog: () -> Unit = {},
    onLongPress: () -> Unit = {},
) {
    Row(
        modifier = modifier
            .fillMaxWidth()
            .background(OhdColors.Bg)
            .padding(horizontal = 16.dp, vertical = 14.dp),
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = name,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 14.sp,
                color = OhdColors.Ink,
            )
            Text(
                text = sub,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                color = OhdColors.Muted,
            )
        }

        val shape = RoundedCornerShape(8.dp)
        // The `Take` / `Taken` button reacts to both a tap (short press →
        // log with defaults) and a long press (→ open the time/dose dialog).
        // `combinedClickable` is foundation-experimental, hence the file-level
        // OptIn at the top of this file.
        when (takenState) {
            TakenState.Pending -> Box(
                modifier = Modifier
                    .width(60.dp)
                    .height(32.dp)
                    .background(OhdColors.Red, shape)
                    .combinedClickable(onClick = onLog, onLongClick = onLongPress),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = "Take",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 12.sp,
                    color = OhdColors.White,
                )
            }
            TakenState.Taken -> Box(
                modifier = Modifier
                    .width(60.dp)
                    .height(32.dp)
                    .background(OhdColors.Bg, shape)
                    .border(BorderStroke(1.dp, OhdColors.Line), shape)
                    .combinedClickable(onClick = onLog, onLongClick = onLongPress),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = "Taken",
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
            }
        }
    }
}
