package com.ohd.emergency.ui.screens

import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.BluetoothSearching
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Wifi
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch

import com.ohd.emergency.data.ApproximateDistance
import com.ohd.emergency.data.CaseVault
import com.ohd.emergency.data.DiscoveredBeacon
import com.ohd.emergency.data.EmergencyRepository
import com.ohd.emergency.data.OperatorSession
import com.ohd.emergency.data.bleScanPermissionName
import com.ohd.emergency.data.hasBleScanPermission
import com.ohd.emergency.ui.components.ChipTone
import com.ohd.emergency.ui.components.EmergencyTopBar
import com.ohd.emergency.ui.components.StatusChip
import com.ohd.emergency.ui.components.SyncIndicatorState

/**
 * Patient discovery — home screen post-login.
 *
 * Per `spec/screens-emergency.md` "Patient discovery screen (paramedic
 * tablet)":
 *
 *     Header: operator label, connection status, GPS-on indicator.
 *     "Scan for nearby OHD users" big primary button. Pressing it
 *     scans BLE for ~10s.
 *     Result list: each row = one detected OHD beacon, with signal
 *     strength, time-since-discovered, and an action button "Request access".
 *     Manual entry option for cases where BLE failed.
 *
 * Layout choices:
 *  - Big "Scan for patients" button uses the primary red — this is the
 *    most-tapped button on the screen and the visual anchor.
 *  - Active-case banner appears at the top if a case is in flight (e.g.
 *    paramedic backed out to discovery without finishing a case);
 *    tapping it returns to the patient view.
 *  - Beacon rows have generous (88dp+) tap targets per gloved-finger
 *    spec.
 */
@Composable
fun DiscoveryScreen(
    onPickBeacon: (DiscoveredBeacon) -> Unit,
    onResumeCase: (caseUlid: String) -> Unit,
    onPanicLogout: () -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()

    var scanning by remember { mutableStateOf(false) }
    val scanResults = remember { MutableStateFlow<List<DiscoveredBeacon>>(emptyList()) }
    val results by scanResults.collectAsState()
    var manualOpen by remember { mutableStateOf(false) }
    var permissionRefused by remember { mutableStateOf(false) }

    // Permission launcher for BLUETOOTH_SCAN (API 31+) /
    // ACCESS_FINE_LOCATION (≤ 30). The launcher is created at composition
    // time; tapping "Scan for patients" without the permission triggers
    // the system dialog. On grant we kick off the scan immediately.
    val permLauncher = rememberLauncherForActivityResult(
        contract = ActivityResultContracts.RequestPermission(),
    ) { granted ->
        permissionRefused = !granted
        if (granted) {
            scanning = true
            scanResults.value = emptyList()
            scope.launch {
                EmergencyRepository.bleScanner().scan().collect { list ->
                    scanResults.value = list
                }
                scanning = false
            }
        }
    }

    val activeCase by CaseVault.activeCase.collectAsState()
    val syncStatus by CaseVault.syncStatus.collectAsState()
    val queuedWrites by CaseVault.queuedWrites.collectAsState()

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(modifier = Modifier.fillMaxSize().padding(contentPadding)) {
            EmergencyTopBar(
                operatorLabel = OperatorSession.operatorLabel(ctx),
                responderLabel = OperatorSession.responderLabel(ctx),
                syncStatus = SyncIndicatorState(syncStatus, queuedWrites.size),
                activeCaseShortLabel = activeCase?.caseUlid?.takeLast(6),
                onPanicLogout = onPanicLogout,
            )

            Column(modifier = Modifier.fillMaxSize().padding(24.dp)) {

                // Active-case banner. Tap to resume.
                val active = activeCase
                if (active != null && !active.handedOff) {
                    Card(
                        modifier = Modifier
                            .fillMaxWidth()
                            .padding(bottom = 16.dp),
                        colors = CardDefaults.cardColors(
                            containerColor = MaterialTheme.colorScheme.primaryContainer,
                        ),
                    ) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(20.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(modifier = Modifier.weight(1f)) {
                                Text(
                                    text = "Active case in progress",
                                    style = MaterialTheme.typography.titleMedium,
                                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                                )
                                Text(
                                    text = "${active.patientLabel} · case ${active.caseUlid.takeLast(6)}",
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onPrimaryContainer,
                                )
                            }
                            Button(
                                onClick = { onResumeCase(active.caseUlid) },
                                colors = ButtonDefaults.buttonColors(
                                    containerColor = MaterialTheme.colorScheme.onPrimary,
                                    contentColor = MaterialTheme.colorScheme.primary,
                                ),
                            ) {
                                Text("Resume")
                            }
                        }
                    }
                }

                // Status chips row.
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    StatusChip(label = "Relay reachable (mock)", tone = ChipTone.Success, outlined = true)
                    StatusChip(label = "GPS off (v0)", tone = ChipTone.Neutral, outlined = true)
                }

                Spacer(Modifier.height(20.dp))

                Text(
                    text = "Scan for nearby OHD patients",
                    style = MaterialTheme.typography.headlineMedium,
                    fontWeight = FontWeight.SemiBold,
                )
                Spacer(Modifier.height(6.dp))
                Text(
                    text = "Bring the tablet close to the patient. Their phone " +
                            "broadcasts an opaque ID; tap a row to send a break-glass request.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )

                Spacer(Modifier.height(20.dp))

                Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Button(
                        onClick = {
                            if (scanning) return@Button
                            // Request the runtime permission first; the
                            // launcher's onResult kicks off the actual
                            // scan once granted. If we already have it
                            // (returning user, MDM-pushed), short-circuit
                            // and start scanning directly.
                            if (hasBleScanPermission(ctx)) {
                                scanning = true
                                scanResults.value = emptyList()
                                scope.launch {
                                    EmergencyRepository.bleScanner().scan().collect { list ->
                                        scanResults.value = list
                                    }
                                    scanning = false
                                }
                            } else {
                                permLauncher.launch(bleScanPermissionName)
                            }
                        },
                        enabled = !scanning,
                        modifier = Modifier.weight(1f).height(72.dp),
                        colors = ButtonDefaults.buttonColors(
                            containerColor = MaterialTheme.colorScheme.primary,
                            contentColor = MaterialTheme.colorScheme.onPrimary,
                        ),
                    ) {
                        Icon(
                            imageVector = Icons.Filled.BluetoothSearching,
                            contentDescription = null,
                        )
                        Text(
                            text = if (scanning) "Scanning…" else "Scan for patients",
                            style = MaterialTheme.typography.titleLarge,
                            modifier = Modifier.padding(start = 12.dp),
                        )
                    }

                    OutlinedButton(
                        onClick = { manualOpen = true },
                        modifier = Modifier.height(72.dp),
                    ) {
                        Icon(imageVector = Icons.Filled.Edit, contentDescription = null)
                        Text(
                            text = "Manual entry",
                            style = MaterialTheme.typography.titleMedium,
                            modifier = Modifier.padding(start = 8.dp),
                        )
                    }
                }

                Spacer(Modifier.height(20.dp))

                if (permissionRefused) {
                    Text(
                        text = "Bluetooth scan permission was refused. Use Manual entry, " +
                                "or grant the permission in system Settings → Apps → OHD Emergency.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.error,
                    )
                    Spacer(Modifier.height(8.dp))
                }

                if (results.isEmpty()) {
                    Text(
                        text = if (scanning) "Looking for OHD beacons…" else "No patients found yet. Tap Scan.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                } else {
                    LazyColumn(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                        items(results) { beacon ->
                            BeaconRow(beacon = beacon, onPick = { onPickBeacon(beacon) })
                        }
                    }
                }
            }
        }

        if (manualOpen) {
            ManualEntryDialog(
                onDismiss = { manualOpen = false },
                onConfirm = { input ->
                    manualOpen = false
                    onPickBeacon(EmergencyRepository.manualBeaconFromInput(input))
                },
            )
        }
    }

    LaunchedEffect(Unit) {
        // Touch-up: leave the scan idle on entry. Paramedic taps Scan
        // when they're physically close to the patient — no auto-scan
        // (battery + privacy: a fleet tablet shouldn't broadcast curiosity
        // every time it's woken).
    }
}

@Composable
private fun BeaconRow(beacon: DiscoveredBeacon, onPick: () -> Unit) {
    Card(
        onClick = onPick,
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(20.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(
                imageVector = Icons.Filled.Wifi,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
            )
            Column(modifier = Modifier.weight(1f).padding(start = 16.dp)) {
                Text(
                    text = beacon.displayLabel ?: "Patient",
                    style = MaterialTheme.typography.titleMedium,
                )
                Text(
                    text = "Beacon ${beacon.beaconId} · ${beacon.rssiDbm} dBm",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            DistanceChip(beacon.approximateDistance)
        }
    }
}

@Composable
private fun DistanceChip(d: ApproximateDistance) {
    val (label, tone) = when (d) {
        ApproximateDistance.VeryClose -> "Very close" to ChipTone.Success
        ApproximateDistance.Close -> "Close" to ChipTone.Info
        ApproximateDistance.Nearby -> "Nearby" to ChipTone.Warning
        ApproximateDistance.Far -> "Far" to ChipTone.Neutral
    }
    StatusChip(label = label, tone = tone, outlined = true)
}

@Composable
private fun ManualEntryDialog(onDismiss: () -> Unit, onConfirm: (String) -> Unit) {
    var input by remember { mutableStateOf("") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Manual entry") },
        text = {
            Column {
                Text(
                    text = "Type the patient ID, beacon ID, or QR-code value the patient gave you. " +
                            "The relay resolves it the same way as a BLE-discovered beacon.",
                    style = MaterialTheme.typography.bodyMedium,
                )
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(
                    value = input,
                    onValueChange = { input = it },
                    label = { Text("Patient / beacon ID") },
                    singleLine = true,
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = { if (input.isNotBlank()) onConfirm(input) },
            ) { Text("Send request") }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}
