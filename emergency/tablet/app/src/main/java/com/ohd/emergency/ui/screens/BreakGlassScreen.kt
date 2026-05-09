package com.ohd.emergency.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.LockOpen
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

import com.ohd.emergency.data.CaseVault
import com.ohd.emergency.data.EmergencyRepository
import com.ohd.emergency.data.OperatorSession
import com.ohd.emergency.ui.components.ChipTone
import com.ohd.emergency.ui.components.StatusChip
import com.ohd.emergency.ui.theme.CountdownStyle

/**
 * Break-glass dialog flow.
 *
 * Two phases on one route:
 *   Phase 1 — confirm screen: shows the operator label, the discovered
 *     beacon, the expected scope (informational), and a big "Send
 *     request" red button.
 *   Phase 2 — countdown screen: shows the patient-side countdown
 *     mirrored back to the responder ("Waiting for patient response —
 *     28s remaining"), with the resolution chip animating in once the
 *     relay reports back.
 *
 * On approve / auto-grant: navigates to /patient/{caseUlid}.
 * On reject: shows error chip + back button to discovery.
 *
 * Per `spec/screens-emergency.md` "Patient discovery screen → Behaviour":
 *
 *     Tapping "Request access" sends the signed emergency request to
 *     the patient's phone. The tablet shows "Waiting for patient
 *     response... 28s" with the same countdown the patient sees.
 *
 * Per `screens-emergency.md` "Designer's handoff notes": the auto-granted
 * indicator must be visually distinct (amber) so the responder knows at
 * a glance that the patient didn't actively approve. We surface this on
 * both this screen (final chip) and the patient view header.
 */
@Composable
fun BreakGlassScreen(
    beaconId: String,
    onApproved: (caseUlid: String) -> Unit,
    onCancelled: () -> Unit,
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()

    val state by CaseVault.breakGlass.collectAsState()
    var ticker by remember { mutableStateOf(0) } // 1 Hz tick for countdown

    Surface(modifier = Modifier.fillMaxSize()) {
        Box(
            modifier = Modifier.fillMaxSize().padding(40.dp),
            contentAlignment = Alignment.Center,
        ) {
            Column(
                modifier = Modifier.widthIn(max = 720.dp),
                verticalArrangement = Arrangement.spacedBy(20.dp),
            ) {
                when (val s = state) {
                    is CaseVault.BreakGlassState.Idle -> {
                        ConfirmPanel(
                            beaconId = beaconId,
                            onSend = {
                                val operator = OperatorSession.operatorLabel(ctx) ?: "Unknown operator"
                                val responder = OperatorSession.responderLabel(ctx) ?: "Unknown responder"
                                CaseVault.startWaiting(
                                    patientBeaconId = beaconId,
                                    operatorLabel = operator,
                                    responderLabel = responder,
                                    timeoutSeconds = 5,             // mock: 5s
                                    patientAllowOnTimeout = true,   // mock: allow on timeout
                                )
                                scope.launch {
                                    val outcome = EmergencyRepository.initiateBreakGlass(
                                        beacon = EmergencyRepository.manualBeaconFromInput(beaconId),
                                        sceneContext = "Mock scene context (v0)",
                                    )
                                    when (outcome) {
                                        is EmergencyRepository.InitiateOutcome.Granted ->
                                            CaseVault.grantApproved(
                                                patientBeaconId = beaconId,
                                                patientLabel = outcome.patientLabel,
                                                caseUlid = outcome.caseUlid,
                                                grantToken = outcome.grantToken,
                                                autoGranted = false,
                                            )

                                        is EmergencyRepository.InitiateOutcome.AutoGranted ->
                                            CaseVault.grantApproved(
                                                patientBeaconId = beaconId,
                                                patientLabel = outcome.patientLabel,
                                                caseUlid = outcome.caseUlid,
                                                grantToken = outcome.grantToken,
                                                autoGranted = true,
                                            )

                                        is EmergencyRepository.InitiateOutcome.Rejected ->
                                            CaseVault.grantRejected(beaconId)

                                        is EmergencyRepository.InitiateOutcome.TimedOut ->
                                            CaseVault.grantTimedOut(beaconId)

                                        is EmergencyRepository.InitiateOutcome.Failed ->
                                            CaseVault.grantTimedOut(beaconId)
                                    }
                                }
                            },
                            onCancel = onCancelled,
                        )
                    }

                    is CaseVault.BreakGlassState.Waiting -> {
                        CountdownPanel(state = s, tick = ticker, onCancel = {
                            CaseVault.resetBreakGlass()
                            onCancelled()
                        })
                    }

                    is CaseVault.BreakGlassState.Granted -> {
                        ResolvedPanel(
                            title = if (s.autoGranted) "Auto-granted via timeout" else "Patient approved",
                            tone = if (s.autoGranted) ChipTone.AutoGrant else ChipTone.Success,
                            description = if (s.autoGranted) {
                                "Patient did not respond within their timeout window. " +
                                        "Their setting is to allow access in this case. " +
                                        "Note: the patient will see this case as auto-granted in their audit."
                            } else {
                                "Patient explicitly approved the break-glass request. Case is open."
                            },
                            primaryLabel = "Open patient view",
                            onPrimary = {
                                onApproved(s.caseUlid)
                                CaseVault.resetBreakGlass()
                            },
                            onCancel = onCancelled,
                        )
                    }

                    is CaseVault.BreakGlassState.Rejected -> {
                        ResolvedPanel(
                            title = "Patient rejected",
                            tone = ChipTone.Critical,
                            description = "Patient explicitly rejected the request. " +
                                    "Fall back to verbal communication, retry once if you've moved closer, " +
                                    "or escalate to dispatch.",
                            primaryLabel = "Back to discovery",
                            onPrimary = {
                                CaseVault.resetBreakGlass()
                                onCancelled()
                            },
                            onCancel = null,
                        )
                    }

                    is CaseVault.BreakGlassState.TimedOut -> {
                        ResolvedPanel(
                            title = "Timed out (refused on timeout)",
                            tone = ChipTone.Warning,
                            description = "Patient's setting is to refuse on timeout. They didn't respond. " +
                                    "Fall back to verbal or escalate.",
                            primaryLabel = "Back to discovery",
                            onPrimary = {
                                CaseVault.resetBreakGlass()
                                onCancelled()
                            },
                            onCancel = null,
                        )
                    }
                }
            }
        }
    }

    // 1Hz ticker for the countdown.
    LaunchedEffect(state) {
        while (state is CaseVault.BreakGlassState.Waiting) {
            delay(1000)
            ticker += 1
        }
    }
}

@Composable
private fun ConfirmPanel(
    beaconId: String,
    onSend: () -> Unit,
    onCancel: () -> Unit,
) {
    val ctx = LocalContext.current
    Column(verticalArrangement = Arrangement.spacedBy(20.dp)) {
        Text(
            text = "Initiate break-glass",
            style = MaterialTheme.typography.displaySmall,
        )
        Text(
            text = "You're about to send a signed emergency-access request to this patient's phone. " +
                    "They have a few seconds to approve or reject. Patient settings may also auto-grant.",
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Card(
            modifier = Modifier.fillMaxWidth(),
            colors = CardDefaults.cardColors(
                containerColor = MaterialTheme.colorScheme.surfaceVariant,
            ),
        ) {
            Column(modifier = Modifier.padding(20.dp)) {
                LabelValue("Operator", OperatorSession.operatorLabel(ctx) ?: "—")
                Spacer(Modifier.height(6.dp))
                LabelValue("Responder", OperatorSession.responderLabel(ctx) ?: "—")
                Spacer(Modifier.height(6.dp))
                LabelValue("Patient beacon", beaconId)
                Spacer(Modifier.height(6.dp))
                LabelValue(
                    "Expected scope",
                    "Allergies, blood type, advance directives, active meds, recent vitals (24h), " +
                            "active diagnoses. Per patient's emergency profile.",
                )
            }
        }

        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(
                onClick = onSend,
                modifier = Modifier.weight(1f).height(72.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                    contentColor = MaterialTheme.colorScheme.onPrimary,
                ),
            ) {
                Icon(imageVector = Icons.Filled.LockOpen, contentDescription = null)
                Text(
                    text = "Send request",
                    style = MaterialTheme.typography.titleLarge,
                    modifier = Modifier.padding(start = 12.dp),
                )
            }
            OutlinedButton(
                onClick = onCancel,
                modifier = Modifier.height(72.dp),
            ) {
                Text("Cancel", style = MaterialTheme.typography.titleMedium)
            }
        }
    }
}

@Composable
private fun CountdownPanel(
    state: CaseVault.BreakGlassState.Waiting,
    tick: Int,
    onCancel: () -> Unit,
) {
    val elapsed = ((System.currentTimeMillis() - state.sentAtMs) / 1000).toInt()
    val remaining = (state.timeoutSeconds - elapsed).coerceAtLeast(0)

    Column(
        verticalArrangement = Arrangement.spacedBy(20.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier.fillMaxWidth(),
    ) {
        StatusChip(label = "Waiting for patient", tone = ChipTone.Info)
        Text(
            text = remaining.toString(),
            style = CountdownStyle,
            color = MaterialTheme.colorScheme.primary,
        )
        Text(
            text = if (state.patientAllowOnTimeout)
                "Auto-granting in ${remaining}s if no response"
            else
                "Will refuse in ${remaining}s if no response",
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(8.dp))
        Text(
            text = "${state.responderLabel} — ${state.operatorLabel}",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Spacer(Modifier.height(20.dp))
        OutlinedButton(onClick = onCancel) {
            Text("Cancel request")
        }
        // tick is referenced only to make the recomposition explicit;
        // remaining is computed from sentAtMs so it stays accurate.
        @Suppress("UNUSED_VARIABLE")
        val _t = tick
    }
}

@Composable
private fun ResolvedPanel(
    title: String,
    tone: ChipTone,
    description: String,
    primaryLabel: String,
    onPrimary: () -> Unit,
    onCancel: (() -> Unit)?,
) {
    Column(verticalArrangement = Arrangement.spacedBy(16.dp)) {
        StatusChip(label = title, tone = tone)
        Text(
            text = title,
            style = MaterialTheme.typography.headlineMedium,
        )
        Text(
            text = description,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Button(
                onClick = onPrimary,
                modifier = Modifier.weight(1f).height(72.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                    contentColor = MaterialTheme.colorScheme.onPrimary,
                ),
            ) {
                Text(primaryLabel, style = MaterialTheme.typography.titleLarge)
            }
            if (onCancel != null) {
                OutlinedButton(
                    onClick = onCancel,
                    modifier = Modifier.height(72.dp),
                ) {
                    Text("Discovery", style = MaterialTheme.typography.titleMedium)
                }
            }
        }
    }
}

@Composable
private fun LabelValue(label: String, value: String) {
    Row {
        Text(
            text = label,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.fillMaxWidth(0.25f),
        )
        Text(
            text = value,
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurface,
        )
    }
}
