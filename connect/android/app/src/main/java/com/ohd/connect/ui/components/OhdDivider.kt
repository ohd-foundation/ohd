package com.ohd.connect.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import com.ohd.connect.ui.theme.OhdColors

/**
 * Horizontal rule — Pencil `jcCm7`.
 *
 * 1 dp `ohd-line`, fill_container width, with 16 dp horizontal padding on
 * the wrapper so it looks "inset" from the screen edges per the Pencil
 * cards/list reference.
 */
@Composable
fun OhdDivider(modifier: Modifier = Modifier) {
    Box(
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp),
    ) {
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(OhdColors.Line),
        )
    }
}
