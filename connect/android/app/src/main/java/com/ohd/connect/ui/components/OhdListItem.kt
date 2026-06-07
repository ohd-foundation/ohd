package com.ohd.connect.ui.components

import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * List row — Pencil `z99kMg`.
 *
 * Horizontal, padding `[v=14, h=16]`, gap 12, alignItems center, fill
 * `ohd-bg`. Used heavily on Recent events, Food results, Measurements.
 *
 * - [primary] is `Inter 14 / 500 / ohd-ink`.
 * - [secondary] (optional) is `Inter 12 / normal / ohd-muted`.
 * - [meta] (optional, right-aligned) is `Inter 14 / normal / ohd-muted`.
 *   Often a Lucide-style "→" / "+" / "›" or a timestamp ("Today 09:14").
 *   Pass as a plain string — the spec calls these out as text.
 * - [leading] is an optional 20–24 dp icon slot rendered before the text.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun OhdListItem(
    primary: String,
    modifier: Modifier = Modifier,
    secondary: String? = null,
    meta: String? = null,
    leading: @Composable (() -> Unit)? = null,
    onClick: (() -> Unit)? = null,
    /**
     * Optional long-press gesture, used by surfaces that want a
     * delete / context affordance (e.g. the food log's "remove this
     * entry I didn't actually eat" flow). When either [onClick] or this
     * is set, the row uses [combinedClickable] under the hood.
     */
    onLongClick: (() -> Unit)? = null,
) {
    val rowModifier = modifier
        .fillMaxWidth()
        .background(OhdColors.Bg)
        .let { base ->
            when {
                onClick == null && onLongClick == null -> base
                else -> base.combinedClickable(
                    onClick = { onClick?.invoke() },
                    onLongClick = onLongClick,
                )
            }
        }
        .padding(horizontal = 16.dp, vertical = 14.dp)

    Row(
        modifier = rowModifier,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (leading != null) {
            Box(
                modifier = Modifier.size(24.dp),
                contentAlignment = Alignment.Center,
            ) { leading() }
        }

        Column(
            modifier = Modifier
                .weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = primary,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 14.sp,
                color = OhdColors.Ink,
            )
            if (secondary != null) {
                Text(
                    text = secondary,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
            }
        }

        if (meta != null) {
            Text(
                text = meta,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 14.sp,
                color = OhdColors.Muted,
            )
        }
    }
}
