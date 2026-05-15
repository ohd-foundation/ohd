package com.ohd.connect.ui.components

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Variants per spec §2 / Pencil `Bk8Xc`/`Vqjiu`/`Y1cID`/`t2Rjme`.
 *
 * - [Primary]:    fill `ohd-red`, label white. Main CTA.
 * - [Ghost]:      transparent, 1.5 dp `ohd-red` border, label `ohd-red`. Inline alt.
 * - [Secondary]:  transparent, 1.5 dp `ohd-line` border, label `ohd-ink`. Neutral.
 * - [Destructive]: fill `ohd-red-dark`, label white. "Revoke" / "Delete".
 */
enum class OhdButtonVariant { Primary, Ghost, Secondary, Destructive }

/**
 * Single button surface for the OHD design system.
 *
 * Height **40 dp**, padding `[h=20, v=0]`, corner `radius-md`, label
 * `Inter 14 / 500`. Either content-sized or `Modifier.fillMaxWidth()`.
 */
@Composable
fun OhdButton(
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    variant: OhdButtonVariant = OhdButtonVariant.Primary,
    enabled: Boolean = true,
) {
    val shape = RoundedCornerShape(8.dp)
    val padding = PaddingValues(horizontal = 20.dp, vertical = 0.dp)
    val sizing = modifier
        .height(40.dp)
        .defaultMinSize(minWidth = 64.dp)

    when (variant) {
        OhdButtonVariant.Primary -> Button(
            onClick = onClick,
            modifier = sizing,
            enabled = enabled,
            shape = shape,
            colors = ButtonDefaults.buttonColors(
                containerColor = OhdColors.Red,
                contentColor = OhdColors.White,
                disabledContainerColor = OhdColors.Red.copy(alpha = 0.4f),
                disabledContentColor = OhdColors.White.copy(alpha = 0.7f),
            ),
            contentPadding = padding,
        ) { ButtonLabel(label) }

        OhdButtonVariant.Destructive -> Button(
            onClick = onClick,
            modifier = sizing,
            enabled = enabled,
            shape = shape,
            colors = ButtonDefaults.buttonColors(
                containerColor = OhdColors.RedDark,
                contentColor = OhdColors.White,
                disabledContainerColor = OhdColors.RedDark.copy(alpha = 0.4f),
                disabledContentColor = OhdColors.White.copy(alpha = 0.7f),
            ),
            contentPadding = padding,
        ) { ButtonLabel(label) }

        OhdButtonVariant.Ghost -> OutlinedButton(
            onClick = onClick,
            modifier = sizing,
            enabled = enabled,
            shape = shape,
            colors = ButtonDefaults.outlinedButtonColors(
                containerColor = OhdColors.Bg,
                contentColor = OhdColors.Red,
                disabledContentColor = OhdColors.Red.copy(alpha = 0.4f),
            ),
            border = BorderStroke(1.5.dp, if (enabled) OhdColors.Red else OhdColors.Red.copy(alpha = 0.4f)),
            contentPadding = padding,
        ) { ButtonLabel(label) }

        OhdButtonVariant.Secondary -> OutlinedButton(
            onClick = onClick,
            modifier = sizing,
            enabled = enabled,
            shape = shape,
            colors = ButtonDefaults.outlinedButtonColors(
                containerColor = OhdColors.Bg,
                contentColor = OhdColors.Ink,
                disabledContentColor = OhdColors.Ink.copy(alpha = 0.4f),
            ),
            border = BorderStroke(1.5.dp, if (enabled) OhdColors.Line else OhdColors.LineSoft),
            contentPadding = padding,
        ) { ButtonLabel(label) }
    }
}

@Composable
private fun ButtonLabel(label: String) {
    Text(
        text = label,
        fontFamily = OhdBody,
        fontWeight = FontWeight.W500,
        fontSize = 14.sp,
    )
}
