package com.ohd.connect.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Section header — Pencil `O0Y1Aj`.
 *
 * Padding `[v=8, h=16]`, fill `ohd-bg`, label `Inter 11 / 500 / ohd-muted`,
 * letter-spacing 2 sp, content forced to uppercase ("QUICK LOG", "RECENT"…).
 */
@Composable
fun OhdSectionHeader(
    text: String,
    modifier: Modifier = Modifier,
) {
    Text(
        text = text.uppercase(),
        fontFamily = OhdBody,
        fontWeight = FontWeight.W500,
        fontSize = 11.sp,
        letterSpacing = 2.sp,
        color = OhdColors.Muted,
        modifier = modifier
            .fillMaxWidth()
            .background(OhdColors.Bg)
            .padding(horizontal = 16.dp, vertical = 8.dp),
    )
}
