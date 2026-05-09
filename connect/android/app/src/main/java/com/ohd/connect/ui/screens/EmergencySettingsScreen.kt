package com.ohd.connect.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import com.ohd.connect.data.EmergencyConfig
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.theme.OhdConnectTheme
import kotlinx.coroutines.launch

/**
 * Emergency / Break-glass settings — patient side only. Mirrors
 * `connect/web/src/pages/settings/EmergencySettingsPage.tsx` and the eight
 * sections in `connect/spec/screens-emergency.md`:
 *   1. Feature toggle (master switch)
 *   2. Discovery — BLE beacon
 *   3. Approval timing — timeout slider + default-on-timeout radio
 *   4. Lock-screen behaviour — full vs basic-info
 *   5. What responders see — history window, per-channel toggles, sensitivity
 *   6. Location — GPS opt-in
 *   7. Trusted authorities — list with add/remove
 *   8. Advanced — bystander-proxy + reset-to-defaults + disable button
 *
 * Persistence: v0 stores via `StorageRepository.{getEmergencyConfig,
 * setEmergencyConfig}` which back onto `EncryptedSharedPreferences`. When
 * storage's `Settings.SetEmergencyConfig` RPC ships, the repository swaps
 * the persistence path; this screen needs no changes (flagged in
 * `STATUS.md`).
 */
@Composable
fun EmergencySettingsScreen(contentPadding: PaddingValues) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()

    var cfg by remember {
        mutableStateOf(StorageRepository.getEmergencyConfig().getOrDefault(EmergencyConfig()))
    }
    var showResetDialog by remember { mutableStateOf(false) }
    var showAddRoot by remember { mutableStateOf(false) }
    var newRootName by remember { mutableStateOf("") }

    fun update(transform: (EmergencyConfig) -> EmergencyConfig) {
        val next = transform(cfg)
        cfg = next
        scope.launch { StorageRepository.setEmergencyConfig(next) }
    }

    val disabled = !cfg.featureEnabled

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(contentPadding)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                text = "Emergency / Break-glass",
                style = MaterialTheme.typography.headlineSmall,
            )
            Text(
                text = "Settings stored locally for v0; the storage Settings.SetEmergencyConfig RPC ships in v0.x and will promote these to the per-user emergency-template grant.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            // --- 1. Feature toggle ----------------------------------------
            Section(
                title = "Emergency access",
                sub = "Let first responders see basic info about you in a medical emergency.",
            ) {
                ToggleRow(
                    title = "Enable emergency access",
                    sub = "When enabled, your phone broadcasts a low-power Bluetooth signal so nearby emergency responders can find your OHD record. They cannot see anything until you (or a timeout) approves.",
                    checked = cfg.featureEnabled,
                    onChange = { v -> update { it.copy(featureEnabled = v) } },
                )
            }

            // --- 2. Discovery (BLE beacon) --------------------------------
            Section(title = "Discovery", disabled = disabled) {
                ToggleRow(
                    title = "Bluetooth beacon",
                    sub = "Broadcasts an opaque ID. No health information leaves your phone via Bluetooth — the beacon only signals 'OHD installed here.' Battery cost is minimal.",
                    checked = cfg.bleBeacon,
                    disabled = disabled,
                    onChange = { v -> update { it.copy(bleBeacon = v) } },
                )
            }

            // --- 3. Approval timing ---------------------------------------
            Section(title = "Approval timing", disabled = disabled) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        text = "Approval timeout: ${cfg.approvalTimeoutSeconds} seconds",
                        style = MaterialTheme.typography.titleSmall,
                    )
                    Text(
                        text = "When a first responder requests emergency access, you have this long to Approve or Reject. After the timeout, the action below applies automatically.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Slider(
                        value = cfg.approvalTimeoutSeconds.toFloat(),
                        onValueChange = { v ->
                            update { it.copy(approvalTimeoutSeconds = v.toInt()) }
                        },
                        valueRange = 10f..300f,
                        steps = ((300 - 10) / 5) - 1,
                        enabled = !disabled,
                    )

                    Spacer(Modifier.height(4.dp))
                    Text(
                        text = "If you don't respond before timeout",
                        style = MaterialTheme.typography.titleSmall,
                    )
                    RadioRow(
                        selected = cfg.defaultOnTimeout == EmergencyConfig.DefaultAction.ALLOW,
                        title = "Allow access (default)",
                        sub = "Better for unconscious users. The responder gets your basic emergency info if you can't react.",
                        disabled = disabled,
                        onSelect = {
                            update { it.copy(defaultOnTimeout = EmergencyConfig.DefaultAction.ALLOW) }
                        },
                    )
                    RadioRow(
                        selected = cfg.defaultOnTimeout == EmergencyConfig.DefaultAction.REFUSE,
                        title = "Refuse access",
                        sub = "Better against malicious requests when you're nearby and unaware. Unconscious-you can't grant access this way.",
                        disabled = disabled,
                        onSelect = {
                            update { it.copy(defaultOnTimeout = EmergencyConfig.DefaultAction.REFUSE) }
                        },
                    )
                }
            }

            // --- 4. Lock-screen behaviour ---------------------------------
            Section(title = "Lock-screen behaviour", disabled = disabled) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        text = "Approval dialog visibility",
                        style = MaterialTheme.typography.titleSmall,
                    )
                    RadioRow(
                        selected = cfg.lockScreenMode == EmergencyConfig.LockScreenMode.FULL,
                        title = "Full dialog above lock screen (default)",
                        sub = "Recommended for emergencies. Anyone who can pick up your phone can see and approve the dialog.",
                        disabled = disabled,
                        onSelect = {
                            update { it.copy(lockScreenMode = EmergencyConfig.LockScreenMode.FULL) }
                        },
                    )
                    RadioRow(
                        selected = cfg.lockScreenMode == EmergencyConfig.LockScreenMode.BASIC_ONLY,
                        title = "Show only basic info on lock screen",
                        sub = "Hides the responder's name and request details until you unlock. Trades emergency convenience for shoulder-surfer protection.",
                        disabled = disabled,
                        onSelect = {
                            update { it.copy(lockScreenMode = EmergencyConfig.LockScreenMode.BASIC_ONLY) }
                        },
                    )
                }
            }

            // --- 5. What responders see -----------------------------------
            Section(title = "What responders see", disabled = disabled) {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(
                        text = "History window",
                        style = MaterialTheme.typography.titleSmall,
                    )
                    Text(
                        text = "How much recent vital-signs history they can see. Even with 0h, they always get current values.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    HistoryWindowRow(
                        current = cfg.historyWindowHours,
                        disabled = disabled,
                        onSelect = { v -> update { it.copy(historyWindowHours = v) } },
                    )

                    Spacer(Modifier.height(8.dp))
                    Text(
                        text = "Per-channel toggles",
                        style = MaterialTheme.typography.titleSmall,
                    )
                    val ch = cfg.channels
                    ToggleRow(
                        "Allergies",
                        "Critical for safe drug administration.",
                        ch.allergies,
                        disabled,
                    ) { v -> update { it.copy(channels = ch.copy(allergies = v)) } }
                    ToggleRow(
                        "Active medications",
                        "Drug interactions and current treatment context.",
                        ch.medications,
                        disabled,
                    ) { v -> update { it.copy(channels = ch.copy(medications = v)) } }
                    ToggleRow(
                        "Blood type",
                        "Transfusion safety.",
                        ch.bloodType,
                        disabled,
                    ) { v -> update { it.copy(channels = ch.copy(bloodType = v)) } }
                    ToggleRow(
                        "Advance directives",
                        "DNR, organ donation preferences.",
                        ch.advanceDirectives,
                        disabled,
                    ) { v -> update { it.copy(channels = ch.copy(advanceDirectives = v)) } }
                    ToggleRow(
                        "Active diagnoses",
                        "Chronic conditions affecting treatment.",
                        ch.diagnoses,
                        disabled,
                    ) { v -> update { it.copy(channels = ch.copy(diagnoses = v)) } }
                    ToggleRow(
                        "Glucose readings",
                        "Important for diabetic emergencies.",
                        ch.glucose,
                        disabled,
                    ) { v -> update { it.copy(channels = ch.copy(glucose = v)) } }
                    ToggleRow(
                        "Heart rate",
                        "Recent HR for arrhythmia / shock assessment.",
                        ch.heartRate,
                        disabled,
                    ) { v -> update { it.copy(channels = ch.copy(heartRate = v)) } }
                    ToggleRow(
                        "Blood pressure",
                        "Recent BP for cardiovascular context.",
                        ch.bloodPressure,
                        disabled,
                    ) { v -> update { it.copy(channels = ch.copy(bloodPressure = v)) } }
                    ToggleRow(
                        "SpO₂",
                        "Oxygen saturation for respiratory emergencies.",
                        ch.spo2,
                        disabled,
                    ) { v -> update { it.copy(channels = ch.copy(spo2 = v)) } }
                    ToggleRow(
                        "Temperature",
                        "Fever / hypothermia assessment.",
                        ch.temperature,
                        disabled,
                    ) { v -> update { it.copy(channels = ch.copy(temperature = v)) } }

                    Spacer(Modifier.height(8.dp))
                    Text(
                        text = "Sensitivity classes",
                        style = MaterialTheme.typography.titleSmall,
                    )
                    Text(
                        text = "Higher-stakes data classes. Defaults are conservative — only general info is shared by default.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    val s = cfg.sensitivity
                    ToggleRow(
                        "General",
                        "Vitals, medications, allergies — typical emergency info. Default ON.",
                        s.general,
                        disabled,
                    ) { v -> update { it.copy(sensitivity = s.copy(general = v)) } }
                    ToggleRow(
                        "Mental health",
                        "Diagnoses, prescriptions. Default OFF — enable if you'd want responders to know.",
                        s.mentalHealth,
                        disabled,
                    ) { v -> update { it.copy(sensitivity = s.copy(mentalHealth = v)) } }
                    ToggleRow(
                        "Substance use",
                        "Default OFF — relevant for overdose / interaction context.",
                        s.substanceUse,
                        disabled,
                    ) { v -> update { it.copy(sensitivity = s.copy(substanceUse = v)) } }
                    ToggleRow(
                        "Sexual health",
                        "Default OFF.",
                        s.sexualHealth,
                        disabled,
                    ) { v -> update { it.copy(sensitivity = s.copy(sexualHealth = v)) } }
                    ToggleRow(
                        "Reproductive",
                        "Some emergencies need reproductive context — consider enabling if pregnant or with body-anatomy concerns.",
                        s.reproductive,
                        disabled,
                    ) { v -> update { it.copy(sensitivity = s.copy(reproductive = v)) } }
                }
            }

            // --- 6. Location ----------------------------------------------
            Section(title = "Location", disabled = disabled) {
                ToggleRow(
                    title = "Share location",
                    sub = "If enabled, your phone shares its current GPS coordinates with the responding emergency authority when access is granted. Useful for ambulance dispatch when you can't say where you are.",
                    checked = cfg.locationShare,
                    disabled = disabled,
                    onChange = { v -> update { it.copy(locationShare = v) } },
                )
            }

            // --- 7. Trusted authorities -----------------------------------
            Section(title = "Trusted authorities", disabled = disabled) {
                Text(
                    text = "Only requests signed by a trusted authority root can trigger the emergency dialog. The OHD Project default root verifies regional EMS / hospital roots; advanced users can pin extra roots.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(8.dp))
                cfg.trustRoots.forEach { root ->
                    Row(
                        modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Column(modifier = Modifier.weight(1f)) {
                            Text(text = root.name, style = MaterialTheme.typography.bodyMedium)
                            Text(
                                text = "scope: ${root.scope}",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        if (root.removable) {
                            OutlinedButton(
                                enabled = !disabled,
                                onClick = {
                                    update { it.copy(trustRoots = it.trustRoots.filter { r -> r.id != root.id }) }
                                },
                            ) { Text("Remove") }
                        } else {
                            Text(
                                "Built-in",
                                style = MaterialTheme.typography.labelSmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
                OutlinedButton(
                    enabled = !disabled,
                    onClick = { showAddRoot = true },
                    modifier = Modifier.fillMaxWidth(),
                ) { Text("+ Add trust root") }
            }

            // --- 8. Advanced ---------------------------------------------
            Section(title = "Advanced", disabled = disabled) {
                ToggleRow(
                    title = "Bystander-proxy role",
                    sub = "Your phone helps forward emergency requests for nearby OHD users who don't have internet. Your phone never sees their data — it just relays encrypted bytes. Disable to opt out of this Good-Samaritan behaviour.",
                    checked = cfg.bystanderProxy,
                    disabled = disabled,
                    onChange = { v -> update { it.copy(bystanderProxy = v) } },
                )
                Spacer(Modifier.height(8.dp))
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    OutlinedButton(
                        enabled = !disabled,
                        onClick = { showResetDialog = true },
                    ) { Text("Reset to defaults") }
                    Button(
                        onClick = { update { it.copy(featureEnabled = false) } },
                        colors = ButtonDefaults.buttonColors(
                            containerColor = MaterialTheme.colorScheme.error,
                            contentColor = MaterialTheme.colorScheme.onError,
                        ),
                    ) { Text("Disable emergency feature") }
                }
            }
            Spacer(Modifier.height(24.dp))
        }
    }

    if (showResetDialog) {
        AlertDialog(
            onDismissRequest = { showResetDialog = false },
            title = { Text("Reset emergency profile?") },
            text = { Text("All channel toggles, sensitivity classes, trust roots and timing settings will be restored to factory defaults.") },
            confirmButton = {
                TextButton(onClick = {
                    cfg = EmergencyConfig()
                    scope.launch { StorageRepository.setEmergencyConfig(cfg) }
                    showResetDialog = false
                }) { Text("Reset") }
            },
            dismissButton = {
                TextButton(onClick = { showResetDialog = false }) { Text("Cancel") }
            },
        )
    }

    if (showAddRoot) {
        AlertDialog(
            onDismissRequest = {
                showAddRoot = false
                newRootName = ""
            },
            title = { Text("Add a trust root") },
            text = {
                Column {
                    Text(
                        text = "Cert validation runs in v0.x. For v0 the entry is name-only.",
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                    Spacer(Modifier.height(8.dp))
                    OutlinedTextField(
                        value = newRootName,
                        onValueChange = { newRootName = it },
                        label = { Text("Trust root name") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth(),
                    )
                }
            },
            confirmButton = {
                TextButton(
                    enabled = newRootName.isNotBlank(),
                    onClick = {
                        val id = "user_${System.currentTimeMillis().toString(36)}"
                        update {
                            it.copy(
                                trustRoots = it.trustRoots + EmergencyConfig.TrustRoot(
                                    id = id,
                                    name = newRootName.trim(),
                                    scope = "user-added",
                                    removable = true,
                                ),
                            )
                        }
                        newRootName = ""
                        showAddRoot = false
                    },
                ) { Text("Add") }
            },
            dismissButton = {
                TextButton(onClick = {
                    showAddRoot = false
                    newRootName = ""
                }) { Text("Cancel") }
            },
        )
    }
}

@Composable
private fun Section(
    title: String,
    sub: String? = null,
    disabled: Boolean = false,
    content: @Composable () -> Unit,
) {
    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            Text(
                text = title,
                style = MaterialTheme.typography.titleMedium,
                color = if (disabled) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
            )
            if (sub != null) {
                Text(
                    text = sub,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            HorizontalDivider(modifier = Modifier.padding(vertical = 8.dp))
            content()
        }
    }
}

@Composable
private fun ToggleRow(
    title: String,
    sub: String? = null,
    checked: Boolean,
    disabled: Boolean = false,
    onChange: (Boolean) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Column(modifier = Modifier.weight(1f)) {
            Text(text = title, style = MaterialTheme.typography.bodyMedium)
            if (sub != null) {
                Text(
                    text = sub,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
        Switch(
            checked = checked,
            onCheckedChange = onChange,
            enabled = !disabled,
        )
    }
}

@Composable
private fun RadioRow(
    selected: Boolean,
    title: String,
    sub: String? = null,
    disabled: Boolean = false,
    onSelect: () -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth().padding(vertical = 2.dp),
        verticalAlignment = Alignment.Top,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        RadioButton(
            selected = selected,
            onClick = onSelect,
            enabled = !disabled,
        )
        Column(modifier = Modifier.weight(1f).padding(top = 12.dp)) {
            Text(text = title, style = MaterialTheme.typography.bodyMedium)
            if (sub != null) {
                Text(
                    text = sub,
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

@Composable
private fun HistoryWindowRow(
    current: Int,
    disabled: Boolean,
    onSelect: (Int) -> Unit,
) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        listOf(0, 3, 12, 24).forEach { hours ->
            OutlinedButton(
                onClick = { onSelect(hours) },
                enabled = !disabled,
                colors = if (current == hours) {
                    ButtonDefaults.outlinedButtonColors(
                        containerColor = MaterialTheme.colorScheme.primaryContainer,
                        contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                    )
                } else {
                    ButtonDefaults.outlinedButtonColors()
                },
                modifier = Modifier.weight(1f),
            ) { Text("${hours}h") }
        }
    }
}

@Preview(showBackground = true, heightDp = 1200)
@Composable
private fun EmergencySettingsScreenPreview() {
    OhdConnectTheme {
        Surface { EmergencySettingsScreen(contentPadding = PaddingValues(0.dp)) }
    }
}
