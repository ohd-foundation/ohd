package com.ohd.connect.ui.components

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
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

/**
 * Card surface — Pencil `eOWkh`.
 *
 * Vertical, padding 16, corner `radius-lg` (12 dp), fill `ohd-bg-elevated`,
 * 1 dp `ohd-line-soft` border, gap 8.
 *
 * - [title] when present is rendered `Inter 15 / 600 / ohd-ink`.
 * - [body] is the slot for downstream content; spec recommends
 *   `Inter 13 / normal / ohd-muted` text but the slot is open-ended.
 */
@Composable
fun OhdCard(
    modifier: Modifier = Modifier,
    title: String? = null,
    body: @Composable () -> Unit,
) {
    val shape = RoundedCornerShape(12.dp)
    Column(
        modifier = modifier
            .fillMaxWidth()
            .background(OhdColors.BgElevated, shape)
            .border(BorderStroke(1.dp, OhdColors.LineSoft), shape)
            .padding(16.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (title != null) {
            Text(
                text = title,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W600,
                fontSize = 15.sp,
                color = OhdColors.Ink,
            )
        }
        body()
    }
}
