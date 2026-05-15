package com.ohd.connect.ui.components

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdMono

/** Donut status — drives the sweep colour per spec §2 / Pencil `xEama`. */
enum class NutriStatus { Ok, Light, Exceeded }

/**
 * Nutrition gauge — Pencil `xEama`.
 *
 * Vertical, gap 6, width 80, height 96, alignItems center.
 *
 * - Top: 76 × 76 donut.
 *   - Track: `ohd-line-soft`.
 *   - Sweep: `ohd-muted` (Ok), `ohd-ink` (Light), `ohd-red` (Exceeded).
 *   - Sweep angle: `360 × (percent / 100)`. Values > 100 still draw a full
 *     ring with the "exceeded" colour applied externally via [status].
 * - Inside: value (e.g. "73g") + `JetBrainsMono 11` percent + small "/110g"
 *   target line.
 * - Bottom label: `Inter 11 / normal / ohd-muted` (Carbs / Protein / Fat /
 *   Sugar).
 */
@Composable
fun OhdNutriGauge(
    label: String,
    value: String,
    target: String,
    percent: Int,
    modifier: Modifier = Modifier,
    status: NutriStatus = NutriStatus.Ok,
) {
    val sweepColor = when (status) {
        NutriStatus.Ok -> OhdColors.Muted
        NutriStatus.Light -> OhdColors.Ink
        NutriStatus.Exceeded -> OhdColors.Red
    }

    Column(
        modifier = modifier.width(80.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Box(
            modifier = Modifier.size(76.dp),
            contentAlignment = Alignment.Center,
        ) {
            Canvas(modifier = Modifier.size(76.dp)) {
                val stroke = 6.dp.toPx()
                val arcSize = Size(size.width - stroke, size.height - stroke)
                val topLeft = Offset(stroke / 2f, stroke / 2f)
                // Track ring.
                drawArc(
                    color = OhdColors.LineSoft,
                    startAngle = -90f,
                    sweepAngle = 360f,
                    useCenter = false,
                    topLeft = topLeft,
                    size = arcSize,
                    style = Stroke(width = stroke),
                )
                // Sweep — clamp to [0, 360]; "exceeded" still draws a full ring.
                val sweep = (percent.coerceAtLeast(0) / 100f * 360f).coerceAtMost(360f)
                drawArc(
                    color = sweepColor,
                    startAngle = -90f,
                    sweepAngle = sweep,
                    useCenter = false,
                    topLeft = topLeft,
                    size = arcSize,
                    style = Stroke(width = stroke),
                )
            }
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.spacedBy(0.dp),
            ) {
                Text(
                    text = value,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 13.sp,
                    color = OhdColors.Ink,
                )
                Text(
                    text = "$percent%",
                    fontFamily = OhdMono,
                    fontWeight = FontWeight.W400,
                    fontSize = 11.sp,
                    color = OhdColors.Muted,
                )
                Text(
                    text = "/$target",
                    fontFamily = OhdMono,
                    fontWeight = FontWeight.W400,
                    fontSize = 9.sp,
                    color = OhdColors.Muted,
                )
            }
        }
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 11.sp,
            color = OhdColors.Muted,
        )
    }
}
