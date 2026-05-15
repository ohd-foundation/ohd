package com.ohd.connect.ui.screens.settings

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.ohd.connect.BuildConfig
import com.ohd.connect.ui.components.OhdTopBar
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors
import com.ohd.connect.ui.theme.OhdMono

/**
 * Destination for one row in the Settings hub. The host (navigation agent)
 * routes each value to the matching sub-screen.
 */
enum class SettingsDestination {
    Storage, Access, Forms, Food, HealthConnect, Activities, Reminders, Cord, About,
}

/**
 * Settings hub — Pencil `qHoLS` (within group `VCokI`), spec §4.5.
 *
 * Top bar (no back, no action), then a sequential list of [SettingsRow]s.
 * Each row: 20 dp Lucide icon (`ohd-ink`) + label `Inter 15 / 500 / ohd-ink`
 * (`fill_container`) + 20 dp `OhdIcons.ChevronRight` (`ohd-muted`), padded
 * `[v=14, h=16]` with a 1 dp `ohd-line-soft` bottom separator.
 */
@Composable
fun SettingsHubScreen(
    contentPadding: PaddingValues,
    onNavigate: (SettingsDestination) -> Unit,
) {
    Column(
        modifier = Modifier
            .fillMaxSize()
            .background(OhdColors.Bg)
            .padding(contentPadding),
    ) {
        OhdTopBar(title = "Settings", onBack = null, action = null)

        SettingsRow(icon = OhdIcons.Database, label = "Storage & Data") {
            onNavigate(SettingsDestination.Storage)
        }
        SettingsRow(icon = OhdIcons.Shield, label = "Profile & Access") {
            onNavigate(SettingsDestination.Access)
        }
        SettingsRow(icon = OhdIcons.FileText, label = "Forms & Measurements") {
            onNavigate(SettingsDestination.Forms)
        }
        SettingsRow(icon = OhdIcons.Utensils, label = "Food & Nutrition") {
            onNavigate(SettingsDestination.Food)
        }
        SettingsRow(icon = OhdIcons.Activity, label = "Health Connect") {
            onNavigate(SettingsDestination.HealthConnect)
        }
        SettingsRow(icon = OhdIcons.Dumbbell, label = "Activities") {
            onNavigate(SettingsDestination.Activities)
        }
        SettingsRow(icon = OhdIcons.Bell, label = "Reminders & Calendar") {
            onNavigate(SettingsDestination.Reminders)
        }
        SettingsRow(icon = OhdIcons.Sparkles, label = "CORD") {
            onNavigate(SettingsDestination.Cord)
        }
        SettingsRow(icon = OhdIcons.FileText, label = "About & licences") {
            onNavigate(SettingsDestination.About)
        }

        // Tiny build-stamp footer so the version is visible without drilling
        // into About. Tappable so beta-testers can copy it for bug reports.
        Text(
            text = "OHD Connect ${BuildConfig.VERSION_NAME} · ${BuildConfig.BUILD_TYPE}",
            fontFamily = OhdMono,
            fontWeight = FontWeight.W400,
            fontSize = 11.sp,
            color = OhdColors.Muted,
            modifier = Modifier
                .fillMaxWidth()
                .clickable { onNavigate(SettingsDestination.About) }
                .padding(horizontal = 16.dp, vertical = 12.dp),
        )
    }
}

/**
 * One Settings row.
 *
 * Pencil §4.5: padding `[v=14, h=16]`, height ~52 dp, fill `ohd-bg`,
 * bottom 1 dp `ohd-line-soft` separator. 20 dp Lucide icon + label
 * (`fill_container`) + 20 dp chevron-right.
 */
@Composable
private fun SettingsRow(
    icon: ImageVector,
    label: String,
    onClick: () -> Unit,
) {
    Column {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .background(OhdColors.Bg)
                .clickable { onClick() }
                .padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Icon(
                imageVector = icon,
                contentDescription = null,
                tint = OhdColors.Ink,
                modifier = Modifier.size(20.dp),
            )
            Text(
                text = label,
                fontFamily = OhdBody,
                fontWeight = FontWeight.W500,
                fontSize = 15.sp,
                color = OhdColors.Ink,
                modifier = Modifier.weight(1f),
            )
            Icon(
                imageVector = OhdIcons.ChevronRight,
                contentDescription = null,
                tint = OhdColors.Muted,
                modifier = Modifier.size(20.dp),
            )
        }
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(OhdColors.LineSoft),
        )
    }
}
