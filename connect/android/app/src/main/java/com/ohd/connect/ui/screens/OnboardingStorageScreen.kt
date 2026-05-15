package com.ohd.connect.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.data.Auth
import com.ohd.connect.ui.components.OhdButton
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.screens._shared.DefaultStorageOptions
import com.ohd.connect.ui.screens._shared.OnDeviceExpandedPanel
import com.ohd.connect.ui.screens._shared.RetentionDialog
import com.ohd.connect.ui.screens._shared.StorageOption
import com.ohd.connect.ui.screens._shared.StorageOptionCard
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdDisplay

/**
 * Storage chooser for the first-run flow — Pencil `eKtkU.png`, spec §4.4.
 *
 * Unlike the in-Settings variant ([com.ohd.connect.ui.screens.settings.StorageSettingsScreen]),
 * this onboarding variant has **no** [com.ohd.connect.ui.components.OhdTopBar].
 * The user sees only the body content with the heading + four option cards +
 * notice strip + Continue CTA.
 *
 * The default selection is `OnDevice`, which expands a panel containing an
 * explainer and a "Keep data for: Forever ▾" retention chip.
 */
@Composable
fun OnboardingStorageScreen(
    onContinue: (StorageOption) -> Unit,
    onClaimExistingAccount: (() -> Unit)? = null,
    contentPadding: PaddingValues = PaddingValues(0.dp),
    modifier: Modifier = Modifier,
    errorMessage: String? = null,
    onErrorDismiss: () -> Unit = {},
) {
    val ctx = LocalContext.current
    // Seed selection from the persisted preference so re-entering onboarding
    // (e.g. user backed out before completing) shows their last pick rather
    // than always resetting to OnDevice (bug #4 — Storage settings out of
    // sync with the onboarding choice).
    val persistedDefaultName = Auth.loadStorageOption(ctx, defaultName = StorageOption.OnDevice.name)
    val persistedDefault = StorageOption.entries.firstOrNull { it.name == persistedDefaultName }
        ?: StorageOption.OnDevice
    var selected by remember { mutableStateOf(persistedDefault) }

    var retention by remember { mutableStateOf(Auth.loadRetentionLimits(ctx)) }
    var dialogOpen by remember { mutableStateOf(false) }

    if (dialogOpen) {
        RetentionDialog(
            initial = retention,
            onDismiss = { dialogOpen = false },
            onApply = { newLimits ->
                Auth.saveRetentionLimits(ctx, newLimits)
                retention = newLimits
                dialogOpen = false
            },
        )
    }

    Column(
        modifier = modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding)
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 20.dp, vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(20.dp),
    ) {
        // Heading.
        Text(
            text = "Where should OHD store your data?",
            fontFamily = OhdDisplay,
            fontWeight = FontWeight.W300,
            fontSize = 22.sp,
            lineHeight = (22 * 1.3).sp,
            color = OhdColors.Ink,
            modifier = Modifier.fillMaxWidth(),
        )

        // Subtitle.
        Text(
            text = "You can change this at any time. Your data is always your " +
                "property regardless of where it lives.",
            fontFamily = OhdBody,
            fontWeight = FontWeight.W400,
            fontSize = 13.sp,
            lineHeight = (13 * 1.5).sp,
            color = OhdColors.Muted,
            modifier = Modifier.fillMaxWidth(),
        )

        // Four option cards.
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            DefaultStorageOptions.forEach { display ->
                StorageOptionCard(
                    display = display,
                    selected = selected == display.option,
                    onSelect = { selected = display.option },
                    expandedContent = if (display.option == StorageOption.OnDevice) {
                        {
                            OnDeviceExpandedPanel(
                                retention = retention,
                                onClickLimit = { dialogOpen = true },
                            )
                        }
                    } else {
                        null
                    },
                )
            }
        }

        // Notice strip.
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(OhdColors.BgElevated, RoundedCornerShape(8.dp))
                .padding(12.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.Top,
        ) {
            Icon(
                imageVector = OhdIcons.ShieldCheck,
                contentDescription = null,
                tint = OhdColors.Muted,
                modifier = Modifier.size(16.dp),
            )
            Text(
                text = "Switching storage later migrates all your data. Nothing is " +
                    "lost. Your data is always exportable as an encrypted OHD " +
                    "archive — easily converted to JSONL. Full format spec in " +
                    "the docs (link coming).",
                fontFamily = OhdBody,
                fontWeight = FontWeight.W400,
                fontSize = 12.sp,
                lineHeight = (12 * 1.5).sp,
                color = OhdColors.Muted,
                modifier = Modifier.weight(1f),
            )
        }

        // Inline error / notice strip from the caller (e.g. "Cloud isn't ready
        // yet — using on-device", or a storage-init failure).
        if (errorMessage != null) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .background(OhdColors.RedTint, RoundedCornerShape(8.dp))
                    .padding(12.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.Top,
            ) {
                Text(
                    text = errorMessage,
                    fontFamily = OhdBody,
                    fontWeight = FontWeight.W500,
                    fontSize = 12.sp,
                    lineHeight = (12 * 1.5).sp,
                    color = OhdColors.Red,
                    modifier = Modifier.weight(1f),
                )
            }
        }

        // Primary CTA.
        OhdButton(
            label = "Continue",
            onClick = {
                onErrorDismiss()
                onContinue(selected)
            },
            modifier = Modifier.fillMaxWidth(),
        )

        // "Already have an account?" — bounce to the claim screen that
        // takes a 16×8 recovery code. Wired by MainActivity to navigate to
        // [com.ohd.connect.ui.screens.ClaimAccountScreen]; defaults to
        // a no-op so the existing preview-mode call site stays valid.
        if (onClaimExistingAccount != null) {
            Box(modifier = Modifier.size(6.dp))
            androidx.compose.material3.TextButton(
                onClick = onClaimExistingAccount,
                modifier = Modifier.fillMaxWidth(),
            ) {
                androidx.compose.material3.Text(
                    text = "Already have an account?  ›",
                    fontFamily = com.ohd.connect.ui.theme.OhdBody,
                    fontWeight = androidx.compose.ui.text.font.FontWeight.W500,
                    fontSize = 13.sp,
                    color = OhdColors.Muted,
                )
            }
        }

        // Bottom breathing room — keeps the CTA off the system gesture bar
        // when this screen is the only thing on screen during onboarding.
        Box(modifier = Modifier.size(8.dp))
    }
}
