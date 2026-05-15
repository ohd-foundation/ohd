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
import com.ohd.connect.ui.icons.OhdIcons
import com.ohd.connect.ui.theme.OhdBody
import com.ohd.connect.ui.theme.OhdColors

/**
 * Tabs in display order — spec §3 / Pencil `QALVh`. The Pencil "tab2" uses
 * the `plus` glyph and label "LOG" (log-entry shortcut), not a generic
 * "Activity". `tab3` is HISTORY, `tab4` is SETTINGS.
 */
enum class OhdTab(internal val label: String, internal val icon: ImageVector) {
    Home(label = "HOME", icon = OhdIcons.Home),
    Log(label = "LOG", icon = OhdIcons.Plus),
    History(label = "HISTORY", icon = OhdIcons.History),
    Settings(label = "SETTINGS", icon = OhdIcons.Settings),
}

/**
 * Bottom tab bar — Pencil `QALVh`.
 *
 * Height **62 dp**, fill `ohd-bg`, top border 1 dp `ohd-line`. Four tabs,
 * each `fill_container` width: HOME / LOG / HISTORY / SETTINGS.
 *
 * Each tab item (per `CbMHS`): vertical, gap 3, justifyContent center,
 * height 62, width 80. Icon **22 dp** Lucide, label `Inter 10 / normal /
 * letterSpacing 0.5`, **uppercase content**. Inactive: icon + label
 * `ohd-muted`. Active: `ohd-red`.
 */
@Composable
fun OhdBottomTabBar(
    currentTab: OhdTab,
    onTabSelected: (OhdTab) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier.fillMaxWidth().background(OhdColors.Bg)) {
        // Top hairline.
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .height(1.dp)
                .background(OhdColors.Line),
        )
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .height(62.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceEvenly,
        ) {
            OhdTab.values().forEach { tab ->
                val active = tab == currentTab
                val tint = if (active) OhdColors.Red else OhdColors.Muted
                Column(
                    modifier = Modifier
                        .weight(1f)
                        .height(62.dp)
                        .clickable { onTabSelected(tab) },
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.Center,
                ) {
                    Icon(
                        imageVector = tab.icon,
                        contentDescription = tab.label,
                        tint = tint,
                        modifier = Modifier.size(22.dp),
                    )
                    Spacer(Modifier.height(3.dp))
                    Text(
                        text = tab.label,
                        fontFamily = OhdBody,
                        fontWeight = FontWeight.W400,
                        fontSize = 10.sp,
                        letterSpacing = 0.5.sp,
                        color = tint,
                    )
                }
            }
        }
    }
}

// -----------------------------------------------------------------------------
// Backwards-compat shim
// -----------------------------------------------------------------------------
//
// `MainActivity.kt` currently references the legacy `BottomTab` enum
// (Log/Dashboard/Grants/Settings) and `OhdBottomBar` composable. Until the
// navigation agent rewrites `MainActivity.kt` against the spec, this shim
// keeps the old call sites compiling while routing them through the new
// component. The legacy `Dashboard`/`Grants` tabs map onto the closest new
// tab so the app still renders something sensible.

@Deprecated(
    message = "Use OhdTab + OhdBottomTabBar. The navigation agent will replace " +
        "MainActivity.kt with the new four-tab model (Home/Log/History/Settings).",
    replaceWith = ReplaceWith("OhdTab"),
)
enum class BottomTab(val route: String, val label: String) {
    Log(route = "log", label = "Log"),
    Dashboard(route = "dashboard", label = "Dashboard"),
    Grants(route = "grants", label = "Grants"),
    Settings(route = "settings", label = "Settings"),
}

@Suppress("DEPRECATION")
@Deprecated(
    message = "Use OhdBottomTabBar. Legacy four-tab variant maintained only to keep " +
        "the v0 MainActivity compiling — the navigation agent will replace it.",
    replaceWith = ReplaceWith("OhdBottomTabBar"),
)
@Composable
fun OhdBottomBar(
    current: BottomTab,
    onSelect: (BottomTab) -> Unit,
) {
    val mapped = when (current) {
        BottomTab.Log -> OhdTab.Log
        BottomTab.Dashboard -> OhdTab.Home
        BottomTab.Grants -> OhdTab.Settings
        BottomTab.Settings -> OhdTab.Settings
    }
    OhdBottomTabBar(
        currentTab = mapped,
        onTabSelected = { newTab ->
            val legacy = when (newTab) {
                OhdTab.Home -> BottomTab.Dashboard
                OhdTab.Log -> BottomTab.Log
                OhdTab.History -> BottomTab.Log
                OhdTab.Settings -> BottomTab.Settings
            }
            onSelect(legacy)
        },
    )
}
