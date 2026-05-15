package com.ohd.connect.ui.screens.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import com.ohd.connect.ui.components.OhdDivider
import com.ohd.connect.ui.components.OhdListItem
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.theme.OhdColors

/**
 * Profile & Access — bridge into the existing operator-flavoured screens.
 *
 * Spec §3 / §4.5 — the migration moves Grants / Pending / Cases / Audit /
 * Emergency / Export from their own tabs into Settings → Profile & Access.
 * This screen is the bridge: it lists six rows, each routing to the
 * existing Compose screen via the matching callback.
 *
 * Each row uses [OhdListItem] with `meta = "›"` (the existing component
 * takes a `String` meta; spec leaves a chevron-glyph as the visual cue
 * until a future `Composable` meta slot lands).
 */
@Composable
fun AccessSettingsScreen(
    contentPadding: PaddingValues,
    onBack: () -> Unit,
    onOpenGrants: () -> Unit,
    onOpenPending: () -> Unit,
    onOpenCases: () -> Unit,
    onOpenAudit: () -> Unit,
    onOpenEmergency: () -> Unit,
    onOpenExport: () -> Unit,
    onOpenRecovery: () -> Unit = {},
    onOpenPlan: () -> Unit = {},
    onOpenIdentities: () -> Unit = {},
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Profile & Access", onBack = onBack)

        LazyColumn(modifier = Modifier.fillMaxSize()) {
            item {
                AccessRow(
                    primary = "Recovery code",
                    secondary = "16×8 fallback for losing this device",
                    onClick = onOpenRecovery,
                )
                OhdDivider()
            }
            item {
                AccessRow(
                    primary = "Plan",
                    secondary = "Free · 7-day retention · upgrade for sync + unlimited",
                    onClick = onOpenPlan,
                )
                OhdDivider()
            }
            item {
                AccessRow(
                    primary = "Linked identities",
                    secondary = "OIDC providers attached to your profile",
                    onClick = onOpenIdentities,
                )
                OhdDivider()
            }
            item {
                AccessRow(
                    primary = "Grants",
                    secondary = "Tokens issued to clinicians, devices, agents",
                    onClick = onOpenGrants,
                )
                OhdDivider()
            }
            item {
                AccessRow(
                    primary = "Pending approvals",
                    secondary = "Writes awaiting your sign-off",
                    onClick = onOpenPending,
                )
                OhdDivider()
            }
            item {
                AccessRow(
                    primary = "Cases",
                    secondary = "Active and closed clinical cases",
                    onClick = onOpenCases,
                )
                OhdDivider()
            }
            item {
                AccessRow(
                    primary = "Audit log",
                    secondary = "Who has read what",
                    onClick = onOpenAudit,
                )
                OhdDivider()
            }
            item {
                AccessRow(
                    primary = "Emergency",
                    secondary = "What break-glass can see",
                    onClick = onOpenEmergency,
                )
                OhdDivider()
            }
            item {
                AccessRow(
                    primary = "Export",
                    secondary = "Snapshot or full archive",
                    onClick = onOpenExport,
                )
            }
        }
    }
}

@Composable
private fun AccessRow(
    primary: String,
    secondary: String,
    onClick: () -> Unit,
) {
    OhdListItem(
        primary = primary,
        secondary = secondary,
        meta = "›", // unicode "›" (single right-pointing angle quotation mark)
        onClick = onClick,
    )
}
