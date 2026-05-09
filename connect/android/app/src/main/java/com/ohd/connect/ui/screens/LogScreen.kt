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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import androidx.compose.foundation.text.KeyboardOptions
import com.ohd.connect.data.EventChannelInput
import com.ohd.connect.data.EventInput
import com.ohd.connect.data.OhdScalar
import com.ohd.connect.data.PutEventOutcome
import com.ohd.connect.data.StorageRepository
import kotlinx.coroutines.launch

/**
 * Log tab — quick-entry sheets for the four highest-frequency event types.
 *
 * This v0 surface mirrors the spec's "every log action must be reachable in
 * 2 taps or fewer" UX rule from `ux-design.md`. The four primary cards each
 * open a Material3 ModalBottomSheet with a minimal form; submitting builds
 * an [EventInput] and calls `StorageRepository.putEvent` (which routes
 * through the uniffi bindings to the Rust core's `Events.PutEvents`).
 *
 * Future polish (out of v0):
 *   - Replace generic numeric input with channel-aware widgets (e.g. a
 *     mg/dL ↔ mmol/L unit toggle for glucose; the storage core already
 *     handles unit conversion server-side via `Registry.ResolveChannel`).
 *   - Symptom log: severity slider per ux-design.md.
 *   - Medication log: pick from a "things at home" saved list.
 *   - Food log: barcode scanner via ML Kit + OpenFoodFacts resolution.
 */

private enum class LogKind(
    val label: String,
    val eventType: String,
    val channelPath: String,
    val unitHint: String,
) {
    Glucose("Glucose", "std.blood_glucose", "value", "mmol/L"),
    HeartRate("Heart rate", "std.heart_rate_resting", "value", "bpm"),
    Temperature("Body temperature", "std.body_temperature", "value", "°C"),
    Medication("Medication taken", "std.medication_dose", "name", "name"),
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun LogScreen(contentPadding: PaddingValues) {
    var sheetKind by remember { mutableStateOf<LogKind?>(null) }
    var lastResult by remember { mutableStateOf<String?>(null) }
    val sheetState = rememberModalBottomSheetState()
    val scope = rememberCoroutineScope()

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(contentPadding)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            Text(
                text = "Log",
                style = MaterialTheme.typography.headlineSmall,
            )
            Spacer(Modifier.height(4.dp))
            Text(
                text = "Quick-entry — pick a category.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(16.dp))

            LazyColumn(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                items(LogKind.values()) { kind ->
                    LogKindCard(kind = kind, onClick = { sheetKind = kind })
                }
            }

            Spacer(Modifier.height(16.dp))
            lastResult?.let { msg ->
                Text(
                    text = msg,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }

    sheetKind?.let { k ->
        ModalBottomSheet(
            onDismissRequest = { sheetKind = null },
            sheetState = sheetState,
        ) {
            QuickEntrySheet(
                kind = k,
                onSubmit = { value, notes ->
                    val outcome = submit(k, value, notes)
                    lastResult = render(outcome)
                    scope.launch { sheetState.hide() }.invokeOnCompletion { sheetKind = null }
                },
                onCancel = {
                    scope.launch { sheetState.hide() }.invokeOnCompletion { sheetKind = null }
                },
            )
        }
    }
}

@Composable
private fun LogKindCard(kind: LogKind, onClick: () -> Unit) {
    Card(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 0.dp),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
        onClick = onClick,
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Text(
                    text = kind.label,
                    style = MaterialTheme.typography.titleMedium,
                )
                Text(
                    text = kind.eventType,
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Text(
                text = "+",
                style = MaterialTheme.typography.headlineMedium,
                color = MaterialTheme.colorScheme.primary,
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun QuickEntrySheet(
    kind: LogKind,
    onSubmit: (value: String, notes: String) -> Unit,
    onCancel: () -> Unit,
) {
    var value by remember { mutableStateOf(TextFieldValue("")) }
    var notes by remember { mutableStateOf(TextFieldValue("")) }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 24.dp, vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text(
            text = "Log ${kind.label.lowercase()}",
            style = MaterialTheme.typography.titleLarge,
        )
        OutlinedTextField(
            value = value,
            onValueChange = { value = it },
            label = { Text(kind.unitHint) },
            singleLine = true,
            keyboardOptions = if (kind == LogKind.Medication) {
                KeyboardOptions(keyboardType = KeyboardType.Text)
            } else {
                KeyboardOptions(keyboardType = KeyboardType.Decimal)
            },
            modifier = Modifier.fillMaxWidth(),
        )
        TextField(
            value = notes,
            onValueChange = { notes = it },
            label = { Text("Notes (optional)") },
            modifier = Modifier.fillMaxWidth(),
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.End,
        ) {
            TextButton(onClick = onCancel) { Text("Cancel") }
            TextButton(onClick = { onSubmit(value.text, notes.text) }) { Text("Log") }
        }
        Spacer(Modifier.height(8.dp))
    }
}

private fun submit(kind: LogKind, value: String, notes: String): PutEventOutcome {
    val scalar: OhdScalar = when (kind) {
        LogKind.Medication -> OhdScalar.Text(value.ifBlank { "(unspecified)" })
        else -> OhdScalar.Real(value.trim().toDoubleOrNull() ?: 0.0)
    }
    val input = EventInput(
        timestampMs = System.currentTimeMillis(),
        eventType = kind.eventType,
        channels = listOf(EventChannelInput(path = kind.channelPath, scalar = scalar)),
        notes = notes.takeIf { it.isNotBlank() },
    )
    return StorageRepository.putEvent(input).getOrElse { e ->
        PutEventOutcome.Error(code = "INTERNAL", message = e.message ?: e::class.simpleName.orEmpty())
    }
}

private fun render(outcome: PutEventOutcome): String = when (outcome) {
    is PutEventOutcome.Committed -> "✓ Committed ${outcome.ulid.take(12)}…"
    is PutEventOutcome.Pending -> "Pending review (${outcome.ulid.take(12)}…)"
    is PutEventOutcome.Error -> "Error ${outcome.code}: ${outcome.message}"
}
