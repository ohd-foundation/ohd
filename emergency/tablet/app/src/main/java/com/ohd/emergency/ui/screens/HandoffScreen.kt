package com.ohd.emergency.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

import com.ohd.emergency.data.CaseVault
import com.ohd.emergency.data.EmergencyRepository
import com.ohd.emergency.data.OperatorSession
import com.ohd.emergency.ui.components.ChipTone
import com.ohd.emergency.ui.components.EmergencyTopBar
import com.ohd.emergency.ui.components.StatusChip
import com.ohd.emergency.ui.components.SyncIndicatorState

/**
 * End-of-call handoff screen.
 *
 * Per `spec/screens-emergency.md` "Handoff":
 *
 *     Tap [End case / Handoff]
 *     Select receiving facility (autocomplete from operator's typical
 *     destinations + manual entry).
 *     Optional handoff summary text (the MCP can draft this).
 *     Confirm.
 *     Backend: opens new case at the receiving facility's authority
 *     (with current case as predecessor), closes current case,
 *     transitions current grant to read-only.
 *     Tablet UI returns to dispatch / next call.
 *
 * Layout:
 *   - Top bar
 *   - Title + intro line
 *   - Current case summary card (patient label, elapsed)
 *   - Receiving-facility list (cards) with manual-entry text field
 *   - Handoff summary note (optional)
 *   - Big "Confirm handoff" red button
 *   - On success: success screen + "Back to discovery" CTA
 *
 * UX choice: receiving-facility list is a column of cards (chunky),
 * not a dropdown. The number of typical destinations per operator is
 * small (3–6); a dropdown adds a tap and hides options. Manual entry
 * is at the bottom of the list as a freeform field.
 */
@Composable
fun HandoffScreen(
    caseUlid: String,
    onComplete: () -> Unit,
    onOpenPatient: () -> Unit,
    onOpenIntervention: () -> Unit,
    onOpenTimeline: () -> Unit,
    onPanicLogout: () -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val activeCase by CaseVault.activeCase.collectAsState()
    val syncStatus by CaseVault.syncStatus.collectAsState()
    val queuedWrites by CaseVault.queuedWrites.collectAsState()

    val knownFacilities = remember { EmergencyRepository.knownReceivingFacilities() }
    var selected by remember { mutableStateOf<String?>(null) }
    var manualEntry by remember { mutableStateOf("") }
    var summaryNote by remember { mutableStateOf("") }
    var inFlight by remember { mutableStateOf(false) }
    var success by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(modifier = Modifier.fillMaxSize().padding(contentPadding)) {
            EmergencyTopBar(
                operatorLabel = OperatorSession.operatorLabel(ctx),
                responderLabel = OperatorSession.responderLabel(ctx),
                syncStatus = SyncIndicatorState(syncStatus, queuedWrites.size),
                activeCaseShortLabel = activeCase?.caseUlid?.takeLast(6),
                onPanicLogout = onPanicLogout,
            )

            val successorUlid = success
            if (successorUlid != null) {
                HandoffSuccess(
                    successorCaseUlid = successorUlid,
                    onDone = onComplete,
                )
            } else {
                Column(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(horizontal = 24.dp, vertical = 16.dp),
                ) {
                    Text(
                        text = "Handoff",
                        style = MaterialTheme.typography.headlineMedium,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        text = "Pick the receiving facility. The relay opens a successor case under " +
                                "their authority; this case becomes read-only on this device.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.height(16.dp))

                    activeCase?.let {
                        Card(
                            modifier = Modifier.fillMaxWidth(),
                            colors = CardDefaults.cardColors(
                                containerColor = MaterialTheme.colorScheme.surfaceVariant,
                            ),
                        ) {
                            Row(
                                modifier = Modifier.fillMaxWidth().padding(16.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Column(modifier = Modifier.weight(1f)) {
                                    Text(
                                        text = "Patient: ${it.patientLabel}",
                                        style = MaterialTheme.typography.titleMedium,
                                    )
                                    Text(
                                        text = "Case ${it.caseUlid.takeLast(8)} · " +
                                                "open ${minutesSinceText(it.openedAtMs)}",
                                        style = MaterialTheme.typography.bodyMedium,
                                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    )
                                }
                                if (it.autoGranted) {
                                    StatusChip(
                                        label = "Auto-granted",
                                        tone = ChipTone.AutoGrant,
                                    )
                                }
                            }
                        }
                    }

                    Spacer(Modifier.height(20.dp))

                    Text(
                        text = "Receiving facility",
                        style = MaterialTheme.typography.titleMedium,
                    )
                    Spacer(Modifier.height(8.dp))
                    Column(
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                        modifier = Modifier.fillMaxWidth(),
                    ) {
                        knownFacilities.forEach { f ->
                            FacilityRow(
                                label = f,
                                selected = selected == f,
                                onSelect = {
                                    selected = f
                                    manualEntry = ""
                                },
                            )
                        }
                    }

                    Spacer(Modifier.height(8.dp))
                    OutlinedTextField(
                        value = manualEntry,
                        onValueChange = {
                            manualEntry = it
                            if (it.isNotBlank()) selected = null
                        },
                        label = { Text("Or type a facility name") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )

                    Spacer(Modifier.height(16.dp))
                    OutlinedTextField(
                        value = summaryNote,
                        onValueChange = { summaryNote = it },
                        label = { Text("Handoff note (optional)") },
                        modifier = Modifier.fillMaxWidth().height(120.dp),
                    )

                    error?.let { msg ->
                        Spacer(Modifier.height(8.dp))
                        StatusChip(label = msg, tone = ChipTone.Critical, outlined = true)
                    }

                    Spacer(Modifier.height(16.dp))
                    Button(
                        enabled = !inFlight && (selected != null || manualEntry.isNotBlank()),
                        onClick = {
                            val target = manualEntry.takeIf { it.isNotBlank() } ?: selected ?: return@Button
                            inFlight = true; error = null
                            scope.launch {
                                val result = EmergencyRepository.handoffCase(
                                    caseUlid = caseUlid,
                                    receivingFacility = target,
                                    summaryNote = summaryNote.takeIf { it.isNotBlank() },
                                )
                                inFlight = false
                                when (result) {
                                    is EmergencyRepository.HandoffOutcome.Success -> {
                                        success = result.successorCaseUlid
                                    }

                                    is EmergencyRepository.HandoffOutcome.Failed -> {
                                        error = "Handoff failed: ${result.message}"
                                    }
                                }
                            }
                        },
                        modifier = Modifier.fillMaxWidth().height(72.dp),
                        colors = ButtonDefaults.buttonColors(
                            containerColor = MaterialTheme.colorScheme.primary,
                            contentColor = MaterialTheme.colorScheme.onPrimary,
                        ),
                    ) {
                        Text(
                            text = if (inFlight) "Handing off…" else "Confirm handoff",
                            style = MaterialTheme.typography.titleLarge,
                        )
                    }
                }

                CaseNavBar(
                    selected = CaseTab.Handoff,
                    onPatient = onOpenPatient,
                    onIntervention = onOpenIntervention,
                    onTimeline = onOpenTimeline,
                    onHandoff = {},
                )
            }
        }
    }
}

@Composable
private fun FacilityRow(label: String, selected: Boolean, onSelect: () -> Unit) {
    Card(
        onClick = onSelect,
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = if (selected)
                MaterialTheme.colorScheme.primaryContainer
            else
                MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Text(
            text = label,
            style = MaterialTheme.typography.titleMedium,
            color = if (selected)
                MaterialTheme.colorScheme.onPrimaryContainer
            else
                MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.padding(20.dp),
        )
    }
}

@Composable
private fun HandoffSuccess(successorCaseUlid: String, onDone: () -> Unit) {
    Box(
        modifier = Modifier.fillMaxSize().padding(40.dp),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            verticalArrangement = Arrangement.spacedBy(20.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            StatusChip(label = "Handoff successful", tone = ChipTone.Success)
            Text(
                text = "Case handed off",
                style = MaterialTheme.typography.headlineLarge,
            )
            Text(
                text = "Successor case: ${successorCaseUlid.takeLast(8)}",
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                text = "This device retains read-only access for billing and records. " +
                        "The receiving facility has authority going forward.",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(16.dp))
            Button(
                onClick = onDone,
                modifier = Modifier.fillMaxWidth(0.6f).height(72.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                    contentColor = MaterialTheme.colorScheme.onPrimary,
                ),
            ) {
                Text(text = "Back to discovery", style = MaterialTheme.typography.titleLarge)
            }
        }
    }
}

private fun minutesSinceText(ms: Long): String {
    val mins = ((System.currentTimeMillis() - ms) / 60_000).toInt()
    return if (mins < 60) "${mins}m" else "${mins / 60}h ${mins % 60}m"
}
