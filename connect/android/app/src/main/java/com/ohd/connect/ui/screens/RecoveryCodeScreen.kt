package com.ohd.connect.ui.screens

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.widget.Toast
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.OhdAccount
import com.ohd.connect.data.OhdAccountStore
import com.ohd.connect.data.RecoveryCode
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.components.OhdButtonVariant
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdMono

/**
 * Recovery code screen — surfaced once right after onboarding mints the
 * Free-tier account, and re-reachable from Settings → Profile → Recovery
 * code so the user can always look the code up.
 *
 * Layout:
 *  - Top bar with a back arrow (only shown in the Settings re-visit; the
 *    one-time onboarding variant passes [showBack] = false and only the
 *    "I saved it" CTA can advance).
 *  - Warning callout explaining what the code is.
 *  - 16-row monospace grid, each row "XXXX XXXX".
 *  - Copy button + "I saved it" primary CTA.
 *
 * Tapping "I saved it" calls [OhdAccountStore.acknowledgeRecovery] which
 * stops the nag notification from firing.
 */
@Composable
fun RecoveryCodeScreen(
    contentPadding: PaddingValues,
    onAcknowledged: () -> Unit,
    onBack: (() -> Unit)? = null,
    title: String = "Your recovery code",
    primaryLabel: String = "I saved it",
) {
    val ctx = LocalContext.current
    var account by remember { mutableStateOf<OhdAccount?>(null) }
    LaunchedEffect(Unit) {
        account = OhdAccountStore.load(ctx)
    }

    val current = account
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding)
            .verticalScroll(rememberScrollState()),
    ) {
        OhdTopBar(title = title, onBack = onBack)

        Spacer(Modifier.height(8.dp))
        Text(
            text = "This 16×8 code is the only way to recover your OHD account if you lose this device and haven't linked an identity. Write it down or store it in a password manager. We can't show it to you again unless you find this screen.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            lineHeight = 19.sp,
            color = OhdColors.Muted,
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 4.dp),
        )

        Spacer(Modifier.height(12.dp))
        if (current == null) {
            Text(
                text = "Loading…",
                fontFamily = OhdBody,
                color = OhdColors.Muted,
                modifier = Modifier.padding(horizontal = 16.dp),
            )
        } else {
            RecoveryGrid(current.recoveryCode)
        }

        Spacer(Modifier.height(16.dp))
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            OhdButton(
                label = "Copy",
                variant = OhdButtonVariant.Ghost,
                onClick = {
                    current?.let { copyToClipboard(ctx, it.recoveryCode) }
                },
                modifier = Modifier.weight(1f),
            )
            OhdButton(
                label = primaryLabel,
                variant = OhdButtonVariant.Primary,
                onClick = {
                    OhdAccountStore.acknowledgeRecovery(ctx)
                    onAcknowledged()
                },
                modifier = Modifier.weight(1f),
            )
        }
        Spacer(Modifier.height(24.dp))
    }
}

@Composable
private fun RecoveryGrid(code: RecoveryCode) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp)
            .background(OhdColors.BgElevated, RoundedCornerShape(10.dp))
            .border(BorderStroke(1.dp, OhdColors.Line), RoundedCornerShape(10.dp))
            .padding(vertical = 12.dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        code.lines.forEachIndexed { idx, _ ->
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 14.dp, vertical = 3.dp),
                horizontalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text(
                    text = String.format("%02d", idx + 1),
                    fontFamily = OhdMono,
                    fontWeight = FontWeight.W400,
                    fontSize = 12.sp,
                    color = OhdColors.Muted,
                )
                Text(
                    text = code.formatRow(idx),
                    fontFamily = OhdMono,
                    fontWeight = FontWeight.W500,
                    fontSize = 14.sp,
                    color = OhdColors.Ink,
                )
            }
        }
    }
}

private fun copyToClipboard(ctx: Context, code: RecoveryCode) {
    val clipboard = ctx.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
    val text = code.lines.indices.joinToString("\n") { code.formatRow(it) }
    clipboard?.setPrimaryClip(ClipData.newPlainText("OHD recovery code", text))
    Toast.makeText(ctx, "Recovery code copied", Toast.LENGTH_SHORT).show()
}
