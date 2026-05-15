package com.ohd.connect.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdDisplay

/**
 * Stat tile — Pencil `A47LgC`.
 *
 * Vertical, padding 16, corner `radius-lg` (12 dp), fill `ohd-bg-elevated`,
 * gap 4. Number on top in `Outfit 32 / 200 / ohd-ink` (e.g. "847", "12,847"),
 * label below in `Inter 12 / normal / ohd-muted`.
 *
 * Width 160 by default; pair two side-by-side via `Modifier.weight(1f)` for
 * the "fill_container" pattern shown on Home v2.
 */
@Composable
fun OhdStatTile(
    value: String,
    label: String,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier = modifier
            .fillMaxWidth()
            .background(OhdColors.BgElevated, RoundedCornerShape(12.dp))
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        Text(
            text = value,
            fontFamily = OhdDisplay,
            fontWeight = FontWeight.W200,
            fontSize = 32.sp,
            color = OhdColors.Ink,
        )
        Text(
            text = label,
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 12.sp,
            color = OhdColors.Muted,
        )
    }
}
