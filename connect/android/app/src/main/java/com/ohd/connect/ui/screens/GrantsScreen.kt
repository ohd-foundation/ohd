package com.ohd.connect.ui.screens

import androidx.compose.animation.AnimatedVisibility
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
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import com.ohd.connect.data.CreateGrantInput
import com.ohd.connect.data.CreateGrantResult
import com.ohd.connect.data.GrantSummary
import com.ohd.connect.data.GrantTemplates
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.theme.OhdConnectTheme
import kotlinx.coroutines.launch

/**
 * Grants tab — real implementation.
 *
 * Sub-tabs: Active grants list / Pending review queue. Floating action
 * button on Active opens a "Create from template" bottom sheet.
 *
 * The list backs `Grants.ListGrants`; the create flow backs `Grants.CreateGrant`;
 * revoke backs `Grants.RevokeGrant`. The repository's uniffi calls are
 * TODO-stubs (see `StorageRepository.kt`); UI is real.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun GrantsScreen(contentPadding: PaddingValues) {
    var sub by remember { mutableStateOf(GrantsSubTab.Active) }
    var grants by remember { mutableStateOf<List<GrantSummary>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }
    var showCreate by remember { mutableStateOf(false) }
    var lastShare by remember { mutableStateOf<CreateGrantResult?>(null) }
    var refreshTick by remember { mutableStateOf(0) }
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()

    LaunchedEffect(refreshTick) {
        StorageRepository.listGrants()
            .onSuccess {
                grants = it
                error = null
            }
            .onFailure { error = "Couldn't load grants: ${it.message}" }
    }

    Scaffold(
        floatingActionButton = {
            if (sub == GrantsSubTab.Active) {
                ExtendedFloatingActionButton(
                    onClick = { showCreate = true },
                    icon = { Icon(Icons.Filled.Add, contentDescription = "New grant") },
                    text = { Text("New grant") },
                    containerColor = MaterialTheme.colorScheme.primary,
                    contentColor = MaterialTheme.colorScheme.onPrimary,
                )
            }
        },
    ) { fabPadding ->
        Surface(modifier = Modifier.fillMaxSize()) {
            Column(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(contentPadding)
                    .padding(fabPadding)
                    .padding(horizontal = 16.dp, vertical = 12.dp),
            ) {
                Text(text = "Grants", style = MaterialTheme.typography.headlineSmall)
                Text(
                    text = "Who can read or write your data, under what scope, for how long.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(12.dp))

                SubTabBar(current = sub, onSelect = { sub = it })

                Spacer(Modifier.height(12.dp))

                when (sub) {
                    GrantsSubTab.Active -> ActivePane(
                        grants = grants,
                        error = error,
                        onRevoke = { ulid, reason ->
                            scope.launch {
                                StorageRepository.revokeGrant(ulid, reason)
                                refreshTick++
                            }
                        },
                    )
                    GrantsSubTab.Pending -> PendingPane(contentPadding = PaddingValues(0.dp))
                }
            }
        }
    }

    if (showCreate) {
        ModalBottomSheet(
            onDismissRequest = { showCreate = false },
            sheetState = sheetState,
        ) {
            CreateGrantSheet(
                onSubmit = { tplId, label, purpose ->
                    val input = GrantTemplates.forTemplate(tplId, label, purpose)
                    StorageRepository.createGrant(input).fold(
                        onSuccess = { share ->
                            lastShare = share
                            refreshTick++
                            android.util.Log.i(
                                "OhdGrants",
                                "createGrant ok: tpl=$tplId label=$label",
                            )
                        },
                        onFailure = { e ->
                            error = "Create grant failed: ${e.message ?: e.javaClass.simpleName}"
                            android.util.Log.e(
                                "OhdGrants",
                                "createGrant failed: tpl=$tplId label=$label",
                                e,
                            )
                        },
                    )
                    scope.launch { sheetState.hide() }
                        .invokeOnCompletion { showCreate = false }
                },
                onCancel = {
                    scope.launch { sheetState.hide() }
                        .invokeOnCompletion { showCreate = false }
                },
            )
        }
    }

    lastShare?.let { share ->
        ModalBottomSheet(
            onDismissRequest = { lastShare = null },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        ) {
            ShareSheet(result = share, onDone = { lastShare = null })
        }
    }
}

private enum class GrantsSubTab(val label: String) {
    Active("Active"),
    Pending("Pending"),
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SubTabBar(current: GrantsSubTab, onSelect: (GrantsSubTab) -> Unit) {
    SingleChoiceSegmentedButtonRow(modifier = Modifier.fillMaxWidth()) {
        GrantsSubTab.entries.forEachIndexed { idx, tab ->
            SegmentedButton(
                shape = SegmentedButtonDefaults.itemShape(idx, GrantsSubTab.entries.size),
                onClick = { onSelect(tab) },
                selected = tab == current,
            ) {
                Text(tab.label)
            }
        }
    }
}

@Composable
private fun ActivePane(
    grants: List<GrantSummary>,
    error: String?,
    onRevoke: (ulid: String, reason: String?) -> Unit,
) {
    if (error != null) {
        Text(text = error, color = MaterialTheme.colorScheme.error)
        return
    }
    if (grants.isEmpty()) {
        Box(
            modifier = Modifier.fillMaxSize().padding(top = 64.dp),
            contentAlignment = Alignment.TopCenter,
        ) {
            Text(
                text = "No active grants.\nTap + to issue one to a doctor, family member, or researcher.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        return
    }
    LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
        items(grants) { g ->
            GrantCard(grant = g, onRevoke = { reason -> onRevoke(g.ulid, reason) })
        }
    }
}

@Composable
private fun GrantCard(grant: GrantSummary, onRevoke: (reason: String?) -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    val nowMs = System.currentTimeMillis()
    val expired = grant.expiresAtMs != null && grant.expiresAtMs < nowMs
    val expiringSoon =
        grant.expiresAtMs != null && !expired && grant.expiresAtMs - nowMs < 7L * 86_400_000L

    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
        modifier = Modifier.fillMaxWidth(),
        onClick = { expanded = !expanded },
    ) {
        Column(modifier = Modifier.padding(horizontal = 16.dp, vertical = 12.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(text = grant.granteeLabel, style = MaterialTheme.typography.titleMedium)
                    Text(
                        text = "${grant.granteeKind} · ${grant.approvalMode}",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                StatusFlag(expired = expired, expiringSoon = expiringSoon, revoked = grant.revokedAtMs != null)
            }
            Spacer(Modifier.height(8.dp))
            Text(
                text = "Expires: " + (grant.expiresAtMs?.let { fmtDate(it) } ?: "indefinite"),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                text = "Use count: ${grant.useCount}" +
                        (grant.lastUsedMs?.let { " · last used " + fmtRelative(it) } ?: ""),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )

            AnimatedVisibility(visible = expanded) {
                Column(modifier = Modifier.padding(top = 8.dp)) {
                    HorizontalDivider()
                    Spacer(Modifier.height(8.dp))
                    Text(
                        text = "Read scope",
                        style = MaterialTheme.typography.labelMedium,
                    )
                    Text(
                        text = grant.readEventTypes.ifEmpty { listOf("(default)") }.joinToString(", "),
                        style = MaterialTheme.typography.bodySmall,
                    )
                    Spacer(Modifier.height(6.dp))
                    Text(
                        text = "Write scope",
                        style = MaterialTheme.typography.labelMedium,
                    )
                    Text(
                        text = grant.writeEventTypes.ifEmpty { listOf("(none)") }.joinToString(", "),
                        style = MaterialTheme.typography.bodySmall,
                    )
                    if (grant.deniedSensitivityClasses.isNotEmpty()) {
                        Spacer(Modifier.height(6.dp))
                        Text(
                            text = "Denied sensitivity classes",
                            style = MaterialTheme.typography.labelMedium,
                        )
                        Text(
                            text = grant.deniedSensitivityClasses.joinToString(", "),
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                    Spacer(Modifier.height(12.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        OutlinedButton(
                            onClick = { /* TODO: per-grant audit screen */ },
                        ) { Text("View audit") }
                        if (!expired && grant.revokedAtMs == null) {
                            Button(
                                onClick = { onRevoke(null) },
                                colors = ButtonDefaults.buttonColors(
                                    containerColor = MaterialTheme.colorScheme.error,
                                    contentColor = MaterialTheme.colorScheme.onError,
                                ),
                            ) { Text("Revoke") }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun StatusFlag(expired: Boolean, expiringSoon: Boolean, revoked: Boolean) {
    val (label, container) = when {
        revoked -> "revoked" to MaterialTheme.colorScheme.errorContainer
        expired -> "expired" to MaterialTheme.colorScheme.errorContainer
        expiringSoon -> "expiring soon" to MaterialTheme.colorScheme.tertiaryContainer
        else -> "active" to MaterialTheme.colorScheme.primaryContainer
    }
    AssistChip(
        onClick = {},
        label = { Text(label, style = MaterialTheme.typography.labelSmall) },
        colors = AssistChipDefaults.assistChipColors(containerColor = container),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun CreateGrantSheet(
    onSubmit: (tplId: GrantTemplates.Id, label: String, purpose: String?) -> Unit,
    onCancel: () -> Unit,
) {
    var template by remember { mutableStateOf(GrantTemplates.Id.PRIMARY_DOCTOR) }
    var label by remember { mutableStateOf("") }
    var purpose by remember { mutableStateOf("") }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 24.dp, vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("Issue a grant", style = MaterialTheme.typography.titleLarge)
        Text(
            "Pick a template — we set sensible defaults. You can fine-tune the scope later.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        // Template picker — vertical list of cards
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            GrantTemplates.Id.entries.forEach { id ->
                Card(
                    onClick = { template = id },
                    colors = CardDefaults.cardColors(
                        containerColor = if (id == template) {
                            MaterialTheme.colorScheme.primaryContainer
                        } else {
                            MaterialTheme.colorScheme.surfaceVariant
                        },
                    ),
                ) {
                    Column(modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
                        Text(id.label, style = MaterialTheme.typography.titleSmall)
                        Text(
                            id.sub,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }

        OutlinedTextField(
            value = label,
            onValueChange = { label = it },
            label = { Text("Grantee label (e.g. Dr Eva Novák)") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
            keyboardOptions = KeyboardOptions.Default,
        )
        OutlinedTextField(
            value = purpose,
            onValueChange = { purpose = it },
            label = { Text("Purpose (optional)") },
            modifier = Modifier.fillMaxWidth(),
        )

        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.End,
        ) {
            TextButton(onClick = onCancel) { Text("Cancel") }
            Button(
                enabled = label.isNotBlank(),
                onClick = { onSubmit(template, label.trim(), purpose.trim().ifBlank { null }) },
            ) { Text("Issue") }
        }
        Spacer(Modifier.height(16.dp))
    }
}

@Composable
private fun ShareSheet(result: CreateGrantResult, onDone: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 24.dp, vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("Share this grant", style = MaterialTheme.typography.titleLarge)
        Text(
            "Save the token now — it's only ever displayed once. Anyone with this token can use the configured scope.",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.tertiary,
        )
        OutlinedTextField(
            value = result.token,
            onValueChange = {},
            readOnly = true,
            label = { Text("Grant token") },
            modifier = Modifier.fillMaxWidth(),
        )
        OutlinedTextField(
            value = result.shareUrl,
            onValueChange = {},
            readOnly = true,
            label = { Text("Share URL") },
            modifier = Modifier.fillMaxWidth(),
        )
        Text(
            "Send via NFC tap, paste into the operator's app, or scan the QR code (TBD).",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.End,
        ) {
            Button(onClick = onDone) { Text("Done") }
        }
        Spacer(Modifier.height(16.dp))
    }
}

@Preview(showBackground = true, heightDp = 720)
@Composable
private fun GrantsScreenPreview() {
    OhdConnectTheme {
        Surface { GrantsScreen(contentPadding = PaddingValues(0.dp)) }
    }
}
