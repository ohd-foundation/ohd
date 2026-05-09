package com.ohd.connect.ui.screens

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
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Checkbox
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import com.ohd.connect.data.PendingSummary
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.theme.OhdConnectTheme
import kotlinx.coroutines.launch

/**
 * Pending review queue. Each row shows the submitter (grant label + ULID
 * tail), event type, key channel preview ("glucose: 6.4 mmol/L") and
 * submitted-at relative time.
 *
 * Actions per row: Approve / Reject / Approve+trust-this-type.
 *
 * Long-press enters bulk-select mode → Approve all / Reject all.
 *
 * Backs `Pending.{ListPending, ApprovePending, RejectPending}`. Repository's
 * uniffi calls are TODO-stubs (see `StorageRepository.kt`). UI is real.
 */
@Composable
fun PendingScreen(contentPadding: PaddingValues) {
    Surface(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(contentPadding)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            Text(text = "Pending", style = MaterialTheme.typography.headlineSmall)
            Text(
                text = "Writes from grant-holders awaiting your review.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(12.dp))
            PendingPane(contentPadding = PaddingValues(0.dp))
        }
    }
}

@Composable
fun PendingPane(contentPadding: PaddingValues) {
    var rows by remember { mutableStateOf<List<PendingSummary>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }
    var refreshTick by remember { mutableStateOf(0) }
    val scope = rememberCoroutineScope()
    val selected = remember { mutableStateMapOf<String, Boolean>() }

    LaunchedEffect(refreshTick) {
        StorageRepository.listPending()
            .onSuccess {
                rows = it
                error = null
            }
            .onFailure { error = "Couldn't load pending: ${it.message}" }
    }

    val anySelected = selected.values.any { it }

    Column(modifier = Modifier.fillMaxSize().padding(contentPadding)) {
        if (anySelected) {
            BulkBar(
                selectedCount = selected.values.count { it },
                onApproveAll = {
                    val ids = selected.filterValues { it }.keys.toList()
                    scope.launch {
                        ids.forEach { StorageRepository.approvePending(it, alsoTrustType = false) }
                        selected.clear()
                        refreshTick++
                    }
                },
                onRejectAll = {
                    val ids = selected.filterValues { it }.keys.toList()
                    scope.launch {
                        ids.forEach { StorageRepository.rejectPending(it, reason = null) }
                        selected.clear()
                        refreshTick++
                    }
                },
                onClear = { selected.clear() },
            )
        }
        val errorMsg = error
        when {
            errorMsg != null -> Text(text = errorMsg, color = MaterialTheme.colorScheme.error)
            rows.isEmpty() -> {
                Box(
                    modifier = Modifier.fillMaxSize().padding(top = 64.dp),
                    contentAlignment = Alignment.TopCenter,
                ) {
                    Text(
                        text = "No pending submissions. Things are quiet.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            else -> {
                LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    items(rows, key = { it.ulid }) { row ->
                        PendingRow(
                            row = row,
                            selectMode = anySelected,
                            isSelected = selected[row.ulid] == true,
                            onToggleSelect = {
                                selected[row.ulid] = !(selected[row.ulid] ?: false)
                            },
                            onApprove = { trust ->
                                scope.launch {
                                    StorageRepository.approvePending(row.ulid, alsoTrustType = trust)
                                    refreshTick++
                                }
                            },
                            onReject = { reason ->
                                scope.launch {
                                    StorageRepository.rejectPending(row.ulid, reason = reason)
                                    refreshTick++
                                }
                            },
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun BulkBar(
    selectedCount: Int,
    onApproveAll: () -> Unit,
    onRejectAll: () -> Unit,
    onClear: () -> Unit,
) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = "$selectedCount selected",
            style = MaterialTheme.typography.titleSmall,
            modifier = Modifier.weight(1f),
        )
        TextButton(onClick = onClear) { Text("Clear") }
        OutlinedButton(onClick = onRejectAll) { Text("Reject all") }
        Button(
            onClick = onApproveAll,
            colors = ButtonDefaults.buttonColors(
                containerColor = MaterialTheme.colorScheme.primary,
            ),
        ) { Text("Approve all") }
    }
}

@Composable
private fun PendingRow(
    row: PendingSummary,
    selectMode: Boolean,
    isSelected: Boolean,
    onToggleSelect: () -> Unit,
    onApprove: (trust: Boolean) -> Unit,
    onReject: (reason: String?) -> Unit,
) {
    var expanded by remember { mutableStateOf(false) }
    var showRejectForm by remember { mutableStateOf(false) }

    Card(
        colors = CardDefaults.cardColors(
            containerColor = if (isSelected) {
                MaterialTheme.colorScheme.primaryContainer
            } else {
                MaterialTheme.colorScheme.surfaceVariant
            },
        ),
        modifier = Modifier.fillMaxWidth(),
        onClick = {
            if (selectMode) onToggleSelect() else expanded = !expanded
        },
    ) {
        Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (selectMode) {
                    Checkbox(
                        checked = isSelected,
                        onCheckedChange = { onToggleSelect() },
                    )
                }
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = prettyEventType(row.eventType),
                        style = MaterialTheme.typography.titleMedium,
                    )
                    Text(
                        text = "from ${row.submittingGrantLabel} · " + fmtRelative(row.submittedAtMs),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                AssistChip(
                    onClick = {},
                    label = { Text(row.status, style = MaterialTheme.typography.labelSmall) },
                    colors = AssistChipDefaults.assistChipColors(
                        containerColor = MaterialTheme.colorScheme.tertiaryContainer,
                    ),
                )
            }

            Spacer(Modifier.height(6.dp))
            Text(
                text = row.keyChannelDisplay,
                style = MaterialTheme.typography.bodyMedium,
            )

            if (expanded && !selectMode) {
                Spacer(Modifier.height(8.dp))
                HorizontalDivider()
                Spacer(Modifier.height(8.dp))

                if (showRejectForm) {
                    var reason by remember { mutableStateOf("") }
                    androidx.compose.material3.OutlinedTextField(
                        value = reason,
                        onValueChange = { reason = it },
                        label = { Text("Reason (optional)") },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Spacer(Modifier.height(8.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        TextButton(onClick = { showRejectForm = false }) { Text("Cancel") }
                        Button(
                            onClick = { onReject(reason.ifBlank { null }) },
                            colors = ButtonDefaults.buttonColors(
                                containerColor = MaterialTheme.colorScheme.error,
                                contentColor = MaterialTheme.colorScheme.onError,
                            ),
                        ) { Text("Confirm reject") }
                    }
                } else {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(onClick = { onApprove(false) }) { Text("Approve") }
                        OutlinedButton(onClick = { onApprove(true) }) {
                            Text("Approve & trust ${prettyEventType(row.eventType)}")
                        }
                        TextButton(
                            onClick = { showRejectForm = true },
                            colors = androidx.compose.material3.ButtonDefaults.textButtonColors(
                                contentColor = MaterialTheme.colorScheme.error,
                            ),
                        ) { Text("Reject") }
                    }
                }
            }
        }
    }
}

@Preview(showBackground = true, heightDp = 720)
@Composable
private fun PendingScreenPreview() {
    OhdConnectTheme {
        Surface { PendingScreen(contentPadding = PaddingValues(0.dp)) }
    }
}
