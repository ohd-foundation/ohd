package com.ohd.emergency.ui.components

import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.PriorityHigh
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp

import com.ohd.emergency.data.CriticalInfo
import com.ohd.emergency.ui.theme.EmergencyPalette

/**
 * The red-bordered card at the top of the patient view.
 *
 * Layout:
 *   - Big "CRITICAL" header (red icon + bold label).
 *   - Allergies (each one as a row, with severity chip if available).
 *   - Blood type (one line).
 *   - Advance directives (each one as a row).
 *   - "Flags at a glance" — anticoagulants, pacemakers, etc.
 *
 * Per the brief: "Critical info above the fold (red-bordered card):
 * allergies, blood type, advance directives, current diagnoses." This
 * component covers the first three; "current diagnoses" lives in a
 * sibling block below the card so the red emphasis stays on
 * always-relevant emergencies (don't give an MI patient a beta-blocker
 * if they're allergic to one).
 */
@Composable
fun CriticalCard(info: CriticalInfo, modifier: Modifier = Modifier) {
    Card(
        modifier = modifier
            .fillMaxWidth()
            .border(
                width = 2.dp,
                color = EmergencyPalette.RedBright,
                shape = RoundedCornerShape(12.dp),
            ),
        shape = RoundedCornerShape(12.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Column(modifier = Modifier.padding(20.dp)) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Icon(
                    imageVector = Icons.Filled.PriorityHigh,
                    contentDescription = null,
                    tint = EmergencyPalette.RedBright,
                )
                Text(
                    text = "CRITICAL",
                    style = MaterialTheme.typography.titleMedium,
                    color = EmergencyPalette.RedBright,
                )
            }

            Spacer(Modifier.height(12.dp))

            // Allergies — most often clinically relevant first.
            if (info.allergies.isNotEmpty()) {
                CriticalRow(label = "Allergies") {
                    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        info.allergies.forEach {
                            Text(
                                text = "• $it",
                                style = MaterialTheme.typography.bodyLarge,
                                color = MaterialTheme.colorScheme.onSurface,
                            )
                        }
                    }
                }
                Spacer(Modifier.height(10.dp))
            }

            // Blood type — single short line.
            if (info.bloodType != null) {
                CriticalRow(label = "Blood type") {
                    Text(
                        text = info.bloodType,
                        style = MaterialTheme.typography.titleLarge,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                }
                Spacer(Modifier.height(10.dp))
            }

            // Advance directives.
            if (info.advanceDirectives.isNotEmpty()) {
                CriticalRow(label = "Advance directives") {
                    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        info.advanceDirectives.forEach {
                            Text(
                                text = "• $it",
                                style = MaterialTheme.typography.bodyLarge,
                                color = MaterialTheme.colorScheme.onSurface,
                            )
                        }
                    }
                }
                Spacer(Modifier.height(10.dp))
            }

            // Flags at a glance — anticoagulants, pacemakers, etc.
            if (info.flagsAtAGlance.isNotEmpty()) {
                CriticalRow(label = "Watch") {
                    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        info.flagsAtAGlance.forEach {
                            Text(
                                text = "• $it",
                                style = MaterialTheme.typography.bodyLarge,
                                color = MaterialTheme.colorScheme.onSurface,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun CriticalRow(label: String, content: @Composable () -> Unit) {
    Row(verticalAlignment = Alignment.Top) {
        Text(
            text = label,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(end = 16.dp).fillMaxWidth(0.25f),
        )
        Column(modifier = Modifier.fillMaxWidth()) { content() }
    }
}
