package com.ohd.connect.ui.components

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Right-side action descriptor for [OhdTopBar].
 */
data class TopBarAction(
    val label: String,
    val onClick: () -> Unit,
    val enabled: Boolean = true,
)

/**
 * Top bar — Pencil `kaowR`.
 *
 * Height **52 dp**, padding `[h=16]`, fill `ohd-bg`, bottom border 1 dp
 * `ohd-line`. Layout: 20 dp Lucide back icon + flexible centered title
 * (`Inter 17 / 500`) + right-side action text (`Inter 15 / 500 / ohd-red`).
 *
 * Back icon hides when [onBack] is null. Action hides when [action] is null.
 */
@Composable
fun OhdTopBar(
    title: String,
    modifier: Modifier = Modifier,
    onBack: (() -> Unit)? = null,
    action: TopBarAction? = null,
) {
    Column(modifier = modifier.fillMaxWidth()) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(52.dp)
                .background(OhdColors.Bg)
                .padding(horizontal = 16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            // Leading slot — back icon or fixed-width spacer to keep the
            // title visually centred even when back is hidden.
            if (onBack != null) {
                Box(
                    modifier = Modifier
                        .size(36.dp)
                        .clickable { onBack() },
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        imageVector = OhdIcons.ArrowLeft,
                        contentDescription = "Back",
                        tint = OhdColors.Ink,
                        modifier = Modifier.size(20.dp),
                    )
                }
            } else {
                Spacer(modifier = Modifier.width(36.dp))
            }

            // Title — flex 1, centered.
            Text(
                text = title,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 17.sp,
                color = OhdColors.Ink,
                textAlign = TextAlign.Center,
                modifier = Modifier.weight(1f),
            )

            // Trailing slot — action label or fixed-width spacer (mirrors leading).
            if (action != null) {
                val actionColor = if (action.enabled) OhdColors.Red else OhdColors.Red.copy(alpha = 0.4f)
                Box(
                    modifier = Modifier
                        .let { if (action.enabled) it.clickable { action.onClick() } else it }
                        .padding(horizontal = 4.dp),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        text = action.label,
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W500,
                        fontSize = 15.sp,
                        color = actionColor,
                    )
                }
            } else {
                Spacer(modifier = Modifier.width(36.dp))
            }
        }

        // Bottom hairline.
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(OhdColors.Line),
        )
    }
}

/** Centered top bar without leading icon — convenience for tab roots. */
@Composable
fun OhdTopBarRoot(
    title: String,
    modifier: Modifier = Modifier,
    action: TopBarAction? = null,
) {
    OhdTopBar(title = title, modifier = modifier, onBack = null, action = action)
}
