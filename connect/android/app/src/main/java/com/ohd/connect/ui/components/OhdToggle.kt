package com.ohd.connect.ui.components

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ohd.connect.ui.theme.OhdColors

/**
 * Toggle switch — Pencil `sDhPx` / `j7xy3C`.
 *
 * 44 × 24 frame, corner radius 12 dp.
 *   - Off: fill `ohd-line` with white knob 18 × 18 at x = 3.
 *   - On:  fill `ohd-red` with knob at x = 23.
 *
 * The knob position is animated with a short tween for visual feedback.
 */
@Composable
fun OhdToggle(
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val knobX by animateDpAsState(
        targetValue = if (checked) 23.dp else 3.dp,
        animationSpec = tween(durationMillis = 160),
        label = "ohd-toggle-knob",
    )
    val trackColor = if (checked) OhdColors.Red else OhdColors.Line

    Box(
        modifier = modifier
            .size(width = 44.dp, height = 24.dp)
            .background(trackColor, RoundedCornerShape(12.dp))
            .clickable { onCheckedChange(!checked) },
    ) {
        Box(
            modifier = Modifier
                .offset(x = knobX, y = 3.dp)
                .size(18.dp)
                .background(OhdColors.White, CircleShape),
        )
    }
}
