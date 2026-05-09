package com.ohd.connect.ui.components

import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.DateRange
import androidx.compose.material.icons.filled.Lock
import androidx.compose.material.icons.filled.Settings

/**
 * Four-tab bottom navigation per `ux-design.md` "Nav: Bottom tab bar.
 * 4–5 tabs (Home, Log, History, Settings)". For OHD Connect the four tabs
 * are Log / Dashboard / Grants / Settings — Grants replaces the generic
 * "Home" because grant management is a first-class user activity in OHD,
 * not a buried settings screen.
 */

enum class BottomTab(val route: String, val label: String) {
    Log(route = "log", label = "Log"),
    Dashboard(route = "dashboard", label = "Dashboard"),
    Grants(route = "grants", label = "Grants"),
    Settings(route = "settings", label = "Settings"),
}

@Composable
fun OhdBottomBar(
    current: BottomTab,
    onSelect: (BottomTab) -> Unit,
) {
    NavigationBar(containerColor = MaterialTheme.colorScheme.surface) {
        BottomTab.values().forEach { tab ->
            NavigationBarItem(
                selected = tab == current,
                onClick = { onSelect(tab) },
                label = { Text(tab.label) },
                icon = {
                    Icon(
                        imageVector = when (tab) {
                            BottomTab.Log -> Icons.Filled.Add
                            BottomTab.Dashboard -> Icons.Filled.DateRange
                            BottomTab.Grants -> Icons.Filled.Lock
                            BottomTab.Settings -> Icons.Filled.Settings
                        },
                        contentDescription = tab.label,
                    )
                },
                colors = NavigationBarItemDefaults.colors(
                    selectedIconColor = MaterialTheme.colorScheme.primary,
                    selectedTextColor = MaterialTheme.colorScheme.primary,
                    indicatorColor = MaterialTheme.colorScheme.surfaceVariant,
                ),
            )
        }
    }
}
