package com.ohd.connect.ui.theme

import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Shapes
import androidx.compose.ui.unit.dp

/**
 * Corner-radius scale per spec §1:
 *   - small      = 4 dp  (chips, swatches)
 *   - medium     = 8 dp  (buttons, inputs, segments)
 *   - large      = 12 dp (cards, panels)
 *   - extraLarge = 16 dp (large cards / sheets)
 */
internal val OhdShapes: Shapes = Shapes(
    extraSmall = RoundedCornerShape(2.dp),
    small = RoundedCornerShape(4.dp),
    medium = RoundedCornerShape(8.dp),
    large = RoundedCornerShape(12.dp),
    extraLarge = RoundedCornerShape(16.dp),
)
