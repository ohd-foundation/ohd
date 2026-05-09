package com.ohd.emergency.ui.screens

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Bloodtype
import androidx.compose.material.icons.filled.Edit
import androidx.compose.material.icons.filled.Favorite
import androidx.compose.material.icons.filled.Medication
import androidx.compose.material.icons.filled.Note
import androidx.compose.material.icons.filled.Speed
import androidx.compose.material.icons.filled.Thermostat
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
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.launch

import com.ohd.emergency.data.CaseVault
import com.ohd.emergency.data.EmergencyRepository
import com.ohd.emergency.data.OperatorSession
import com.ohd.emergency.ui.components.EmergencyTopBar
import com.ohd.emergency.ui.components.QuickEntryCard
import com.ohd.emergency.ui.components.SyncIndicatorState
import com.ohd.emergency.ui.components.VitalsNumberPad
import com.ohd.emergency.ui.components.appendDigit
import com.ohd.emergency.ui.components.backspaceDigit

/**
 * Intervention logging — the most-used screen during a case.
 *
 * Layout (landscape primary):
 *   - Top bar
 *   - Two-column layout:
 *       - Left: Quick-entry cards (Vitals / Drug / Observation / Note)
 *       - Right: The currently-selected entry's input pad
 *   - Bottom CaseNavBar
 *
 * Phone-portrait fallback collapses to a single column (cards row,
 * then pad below).
 *
 * UX choice: chunky number pad (custom, see VitalsPad.kt), NOT the
 * system soft keyboard. Per the brief:
 *
 *     Each is a card with chunky number-pad input (NOT a soft keyboard)
 *     for vitals.
 *
 * Drugs and observations use a TextField (system keyboard) because
 * those are textual and a custom alpha keyboard would be over-engineering.
 *
 * Submission flow: tapping "Submit" calls
 * `EmergencyRepository.submitIntervention(...)` which appends to the
 * [CaseVault] queue. The toast (snackbar) below confirms; the timeline
 * tab picks the new event up immediately.
 */
@Composable
fun InterventionScreen(
    caseUlid: String,
    onOpenPatient: () -> Unit,
    onOpenTimeline: () -> Unit,
    onOpenHandoff: () -> Unit,
    onPanicLogout: () -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val activeCase by CaseVault.activeCase.collectAsState()
    val syncStatus by CaseVault.syncStatus.collectAsState()
    val queuedWrites by CaseVault.queuedWrites.collectAsState()

    var category by remember { mutableStateOf(InterventionCategory.HeartRate) }
    var lastSubmitted by remember { mutableStateOf<String?>(null) }

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(modifier = Modifier.fillMaxSize().padding(contentPadding)) {
            EmergencyTopBar(
                operatorLabel = OperatorSession.operatorLabel(ctx),
                responderLabel = OperatorSession.responderLabel(ctx),
                syncStatus = SyncIndicatorState(syncStatus, queuedWrites.size),
                activeCaseShortLabel = activeCase?.caseUlid?.takeLast(6),
                onPanicLogout = onPanicLogout,
            )

            // Two-column layout. Compose handles the breakpoint via
            // `Row` collapsing to vertical when constraint width
            // is small — but for the v0 we just lay out as two
            // columns. Phone-portrait users will scroll horizontally
            // marginally; addressed in later WindowSizeClass branching.
            Row(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(20.dp),
                horizontalArrangement = Arrangement.spacedBy(20.dp),
            ) {
                Column(
                    modifier = Modifier.weight(1f).verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    Text(
                        text = "What did you just do?",
                        style = MaterialTheme.typography.headlineMedium,
                        fontWeight = FontWeight.SemiBold,
                    )
                    InterventionCategory.values().forEach { c ->
                        QuickEntryCard(
                            title = c.cardTitle,
                            subtitle = c.cardSubtitle,
                            leadingIcon = c.icon,
                            onClick = { category = c },
                            selected = category == c,
                        )
                    }
                }

                Column(
                    modifier = Modifier.weight(1.4f).verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    InterventionInput(
                        category = category,
                        onSubmit = { summary, payload, kind ->
                            scope.launch {
                                EmergencyRepository.submitIntervention(
                                    kind = kind,
                                    summary = summary,
                                    payload = payload,
                                )
                                lastSubmitted = summary
                            }
                        },
                    )
                    lastSubmitted?.let { text ->
                        SubmittedToast(text = text)
                    }
                }
            }

            CaseNavBar(
                selected = CaseTab.Intervention,
                onPatient = onOpenPatient,
                onIntervention = {},
                onTimeline = onOpenTimeline,
                onHandoff = onOpenHandoff,
            )
        }
    }
}

private enum class InterventionCategory(
    val cardTitle: String,
    val cardSubtitle: String?,
    val icon: androidx.compose.ui.graphics.vector.ImageVector,
) {
    HeartRate("Heart rate", "bpm — number pad", Icons.Filled.Favorite),
    BloodPressure("Blood pressure", "systolic / diastolic", Icons.Filled.Bloodtype),
    SpO2("SpO2", "%", Icons.Filled.Speed),
    Temperature("Temperature", "°C", Icons.Filled.Thermostat),
    Drug("Drug administered", "name + dose + route", Icons.Filled.Medication),
    Observation("Observation", "free text", Icons.Filled.Edit),
    Note("Note", "free text", Icons.Filled.Note),
}

@Composable
private fun InterventionInput(
    category: InterventionCategory,
    onSubmit: (
        summary: String,
        payload: CaseVault.InterventionPayload,
        kind: CaseVault.InterventionKind,
    ) -> Unit,
) {
    when (category) {
        InterventionCategory.HeartRate -> NumberPadEntry(
            label = "Heart rate",
            unit = "bpm",
            allowDecimal = false,
            channel = "vital.hr",
            kind = CaseVault.InterventionKind.Vital,
            onSubmit = onSubmit,
        )

        InterventionCategory.BloodPressure -> BpEntry(onSubmit = onSubmit)

        InterventionCategory.SpO2 -> NumberPadEntry(
            label = "SpO2",
            unit = "%",
            allowDecimal = false,
            channel = "vital.spo2",
            kind = CaseVault.InterventionKind.Vital,
            onSubmit = onSubmit,
        )

        InterventionCategory.Temperature -> NumberPadEntry(
            label = "Temperature",
            unit = "°C",
            allowDecimal = true,
            channel = "vital.temp",
            kind = CaseVault.InterventionKind.Vital,
            onSubmit = onSubmit,
        )

        InterventionCategory.Drug -> DrugEntry(onSubmit = onSubmit)

        InterventionCategory.Observation -> TextEntry(
            label = "Observation",
            placeholder = "Chief complaint, level of consciousness, skin colour, …",
            kind = CaseVault.InterventionKind.Observation,
            payload = { txt -> CaseVault.InterventionPayload.Observation(freeText = txt) },
            onSubmit = onSubmit,
        )

        InterventionCategory.Note -> TextEntry(
            label = "Note",
            placeholder = "Free text — handoff hints, scene context, …",
            kind = CaseVault.InterventionKind.Note,
            payload = { txt -> CaseVault.InterventionPayload.Note(text = txt) },
            onSubmit = onSubmit,
        )
    }
}

@Composable
private fun NumberPadEntry(
    label: String,
    unit: String,
    allowDecimal: Boolean,
    channel: String,
    kind: CaseVault.InterventionKind,
    onSubmit: (String, CaseVault.InterventionPayload, CaseVault.InterventionKind) -> Unit,
) {
    var value by remember(label) { mutableStateOf("") }
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Column(modifier = Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            VitalsNumberPad(
                currentValue = value,
                label = label,
                unit = unit,
                allowDecimal = allowDecimal,
                onAppend = { c -> value = appendDigit(value, c) },
                onBackspace = { value = backspaceDigit(value) },
            )
            Button(
                enabled = value.isNotEmpty(),
                onClick = {
                    val v = value.toDoubleOrNull() ?: return@Button
                    onSubmit(
                        "$label $v $unit",
                        CaseVault.InterventionPayload.Vital(
                            channel = channel,
                            value = v,
                            unit = unit,
                        ),
                        kind,
                    )
                    value = ""
                },
                modifier = Modifier.fillMaxWidth().height(64.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                    contentColor = MaterialTheme.colorScheme.onPrimary,
                ),
            ) {
                Text("Submit", style = MaterialTheme.typography.titleLarge)
            }
        }
    }
}

@Composable
private fun BpEntry(
    onSubmit: (String, CaseVault.InterventionPayload, CaseVault.InterventionKind) -> Unit,
) {
    var sys by remember { mutableStateOf("") }
    var dia by remember { mutableStateOf("") }
    var editing by remember { mutableStateOf(BpField.Systolic) }

    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Column(modifier = Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                BpReadout(
                    label = "Systolic",
                    value = sys,
                    selected = editing == BpField.Systolic,
                    onSelect = { editing = BpField.Systolic },
                    modifier = Modifier.weight(1f),
                )
                BpReadout(
                    label = "Diastolic",
                    value = dia,
                    selected = editing == BpField.Diastolic,
                    onSelect = { editing = BpField.Diastolic },
                    modifier = Modifier.weight(1f),
                )
            }

            VitalsNumberPad(
                currentValue = if (editing == BpField.Systolic) sys else dia,
                label = if (editing == BpField.Systolic) "Systolic" else "Diastolic",
                unit = "mmHg",
                allowDecimal = false,
                onAppend = { c ->
                    if (editing == BpField.Systolic) sys = appendDigit(sys, c, maxLen = 3)
                    else dia = appendDigit(dia, c, maxLen = 3)
                },
                onBackspace = {
                    if (editing == BpField.Systolic) sys = backspaceDigit(sys)
                    else dia = backspaceDigit(dia)
                },
            )

            Button(
                enabled = sys.isNotEmpty() && dia.isNotEmpty(),
                onClick = {
                    val s = sys.toIntOrNull() ?: return@Button
                    val d = dia.toIntOrNull() ?: return@Button
                    onSubmit(
                        "BP $s/$d mmHg",
                        CaseVault.InterventionPayload.BloodPressure(systolic = s, diastolic = d),
                        CaseVault.InterventionKind.Vital,
                    )
                    sys = ""; dia = ""; editing = BpField.Systolic
                },
                modifier = Modifier.fillMaxWidth().height(64.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                    contentColor = MaterialTheme.colorScheme.onPrimary,
                ),
            ) {
                Text("Submit BP", style = MaterialTheme.typography.titleLarge)
            }
        }
    }
}

private enum class BpField { Systolic, Diastolic }

@Composable
private fun BpReadout(
    label: String,
    value: String,
    selected: Boolean,
    onSelect: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Card(
        onClick = onSelect,
        modifier = modifier,
        colors = CardDefaults.cardColors(
            containerColor = if (selected)
                MaterialTheme.colorScheme.primaryContainer
            else
                MaterialTheme.colorScheme.surface,
        ),
    ) {
        Column(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                text = label,
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                text = value.ifEmpty { "—" },
                style = MaterialTheme.typography.displaySmall,
                color = if (selected) MaterialTheme.colorScheme.onPrimaryContainer
                else MaterialTheme.colorScheme.onSurface,
            )
        }
    }
}

@Composable
private fun DrugEntry(
    onSubmit: (String, CaseVault.InterventionPayload, CaseVault.InterventionKind) -> Unit,
) {
    var name by remember { mutableStateOf("") }
    var dose by remember { mutableStateOf("") }
    var unit by remember { mutableStateOf("mg") }
    var route by remember { mutableStateOf("IV") }

    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Column(modifier = Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(
                value = name,
                onValueChange = { name = it },
                label = { Text("Drug name") },
                singleLine = true,
                modifier = Modifier.fillMaxWidth(),
            )
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedTextField(
                    value = dose,
                    onValueChange = { dose = it },
                    label = { Text("Dose") },
                    singleLine = true,
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Decimal),
                    modifier = Modifier.weight(1f),
                )
                OutlinedTextField(
                    value = unit,
                    onValueChange = { unit = it },
                    label = { Text("Unit") },
                    singleLine = true,
                    modifier = Modifier.weight(0.6f),
                )
                OutlinedTextField(
                    value = route,
                    onValueChange = { route = it },
                    label = { Text("Route") },
                    singleLine = true,
                    modifier = Modifier.weight(0.6f),
                )
            }
            Button(
                enabled = name.isNotBlank() && dose.isNotBlank(),
                onClick = {
                    val d = dose.toDoubleOrNull() ?: return@Button
                    onSubmit(
                        "$name $d $unit $route",
                        CaseVault.InterventionPayload.Drug(
                            name = name.trim(),
                            doseValue = d,
                            doseUnit = unit.trim(),
                            route = route.trim(),
                        ),
                        CaseVault.InterventionKind.Drug,
                    )
                    name = ""; dose = ""
                },
                modifier = Modifier.fillMaxWidth().height(64.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                    contentColor = MaterialTheme.colorScheme.onPrimary,
                ),
            ) {
                Text("Record drug", style = MaterialTheme.typography.titleLarge)
            }
        }
    }
}

@Composable
private fun TextEntry(
    label: String,
    placeholder: String,
    kind: CaseVault.InterventionKind,
    payload: (String) -> CaseVault.InterventionPayload,
    onSubmit: (String, CaseVault.InterventionPayload, CaseVault.InterventionKind) -> Unit,
) {
    var text by remember { mutableStateOf("") }
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Column(modifier = Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedTextField(
                value = text,
                onValueChange = { text = it },
                label = { Text(label) },
                placeholder = { Text(placeholder) },
                modifier = Modifier.fillMaxWidth().height(160.dp),
            )
            Button(
                enabled = text.isNotBlank(),
                onClick = {
                    onSubmit(text.trim(), payload(text.trim()), kind)
                    text = ""
                },
                modifier = Modifier.fillMaxWidth().height(64.dp),
                colors = ButtonDefaults.buttonColors(
                    containerColor = MaterialTheme.colorScheme.primary,
                    contentColor = MaterialTheme.colorScheme.onPrimary,
                ),
            ) {
                Text("Submit $label", style = MaterialTheme.typography.titleLarge)
            }
        }
    }
}

@Composable
private fun SubmittedToast(text: String) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.primaryContainer,
        ),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = "Logged: $text",
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onPrimaryContainer,
                modifier = Modifier.weight(1f),
            )
            Text(
                text = "Visible on timeline",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onPrimaryContainer,
            )
        }
    }
}
