package com.ohd.connect.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
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

/** 4-state time range selector — Home v2 segmented control (Pencil `x8nPv`). */
enum class TimeRange(val label: String) {
    Today(label = "Today"),
    Week(label = "Week"),
    Month(label = "Month"),
    Year(label = "Year"),
}

/**
 * Segmented time-range — Pencil `x8nPv`.
 *
 * Container: height 32 dp, corner `radius-md`, fill `ohd-line-soft`.
 * Each of 4 segments fills its slot of the row equally.
 *
 *  - Active:   fill `ohd-ink`, label `Inter 12 / 500 / #FFFFFF`.
 *  - Inactive: transparent, label `Inter 12 / normal / ohd-muted`.
 */
@Composable
fun OhdSegmentedTimeRange(
    selected: TimeRange,
    onSelect: (TimeRange) -> Unit,
    modifier: Modifier = Modifier,
) {
    val outerShape = RoundedCornerShape(8.dp)
    Row(
        modifier = modifier
            .fillMaxWidth()
            .height(32.dp)
            .background(OhdColors.LineSoft, outerShape)
            .padding(2.dp),
        horizontalArrangement = Arrangement.spacedBy(0.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        TimeRange.values().forEach { range ->
            val active = range == selected
            val segShape = RoundedCornerShape(6.dp)
            val bg = if (active) OhdColors.Ink else OhdColors.LineSoft
            val labelColor = if (active) OhdColors.White else OhdColors.Muted
            val weight = if (active) FontWeight.W500 else FontWeight.W400

            Box(
                modifier = Modifier
                    .weight(1f)
                    .height(28.dp)
                    .background(bg, segShape)
                    .clickable { onSelect(range) },
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    text = range.label,
                    fontFamily = OhdBody,
                    fontWeight = weight,
                    fontSize = 12.sp,
                    color = labelColor,
                )
            }
        }
    }
}
