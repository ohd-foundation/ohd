package com.ohd.emergency.ui.screens

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Assignment
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.LocalHospital
import androidx.compose.material.icons.filled.Person
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.NavigationBarItemDefaults
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

/**
 * Per-case bottom navigation bar.
 *
 * Visible only while a case is active (patient / intervention / timeline
 * / handoff). The nav bar is intentionally case-scoped — there is no
 * "Settings" or "Home" tab here; the paramedic returns to discovery
 * by handing off the case (or via panic logout).
 *
 * UX: 4 tabs, each with a chunky icon + label. Selected tab gets the
 * primary red. Handoff is on the right by convention (action that ends
 * the case = end of the row).
 */
enum class CaseTab(val label: String) {
    Patient("Patient"),
    Intervention("Log"),
    Timeline("Timeline"),
    Handoff("Handoff"),
}

@Composable
fun CaseNavBar(
    selected: CaseTab,
    onPatient: () -> Unit,
    onIntervention: () -> Unit,
    onTimeline: () -> Unit,
    onHandoff: () -> Unit,
) {
    NavigationBar(
        containerColor = MaterialTheme.colorScheme.surface,
        modifier = Modifier
            .fillMaxWidth()
            .height(76.dp),
    ) {
        NavItem(
            tab = CaseTab.Patient,
            selected = selected == CaseTab.Patient,
            onSelect = onPatient,
            icon = Icons.Filled.Person,
        )
        NavItem(
            tab = CaseTab.Intervention,
            selected = selected == CaseTab.Intervention,
            onSelect = onIntervention,
            icon = Icons.Filled.Edit,
        )
        NavItem(
            tab = CaseTab.Timeline,
            selected = selected == CaseTab.Timeline,
            onSelect = onTimeline,
            icon = Icons.Filled.Assignment,
        )
        NavItem(
            tab = CaseTab.Handoff,
            selected = selected == CaseTab.Handoff,
            onSelect = onHandoff,
            icon = Icons.Filled.LocalHospital,
        )
    }
}

@Composable
private fun androidx.compose.foundation.layout.RowScope.NavItem(
    tab: CaseTab,
    selected: Boolean,
    onSelect: () -> Unit,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
) {
    NavigationBarItem(
        selected = selected,
        onClick = onSelect,
        icon = {
            Icon(
                imageVector = icon,
                contentDescription = tab.label,
            )
        },
        label = { Text(text = tab.label, style = MaterialTheme.typography.labelLarge) },
        colors = NavigationBarItemDefaults.colors(
            selectedIconColor = MaterialTheme.colorScheme.primary,
            selectedTextColor = MaterialTheme.colorScheme.primary,
            indicatorColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    )
}
