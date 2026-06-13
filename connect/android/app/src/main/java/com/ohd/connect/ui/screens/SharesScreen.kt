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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
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
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import com.ohd.connect.data.EmergencyConfig
import com.ohd.connect.data.GrantTemplates
import com.ohd.connect.data.GrantTokenStore
import com.ohd.connect.data.ShareKind
import com.ohd.connect.data.ShareRow
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.theme.OhdConnectTheme
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Shares — first-class top-level tab.
 *
 * Implements the Connect side of `cord/spec/data-link.md` §"Connect UI: a
 * first-class Shares tab". Sharing used to be buried inside "Profile &
 * Access"; it is one of the most important things the app does, so it gets
 * its own tab.
 *
 * The screen is a list. Each row carries a label (grantee name), a type chip
 * (doctor / family / researcher / agent / emergency), a status line (scope
 * summary, expiry, last access), and a quick enable/disable toggle that
 * instantly suspends/resumes the share without deleting it (storage flips the
 * grant's `suspended_at_ms`).
 *
 * The emergency break-glass profile is modelled as a pre-configured share,
 * `kind = emergency`, pinned to the top of the list, non-deletable. Its
 * toggle mirrors the emergency feature master switch.
 *
 * Tapping a row opens [ShareDetailScreen] via [onOpenShare].
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SharesScreen(
    contentPadding: PaddingValues,
    onOpenShare: (shareId: String) -> Unit,
) {
    val ctx = LocalContext.current
    var shares by remember { mutableStateOf<List<ShareRow>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }
    var refreshTick by remember { mutableStateOf(0) }
    var showCreate by remember { mutableStateOf(false) }
    var lastToken by remember { mutableStateOf<String?>(null) }
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()

    LaunchedEffect(refreshTick) {
        // getEmergencyConfig + listGrants are blocking network RPCs against
        // remote storage — run them off the main thread. Snapshot-state
        // assignments below are thread-safe.
        withContext(Dispatchers.IO) {
            val emergencyCfg = StorageRepository.getEmergencyConfig()
                .getOrDefault(EmergencyConfig())
            StorageRepository.listGrants(includeRevoked = true)
                .onSuccess { grants ->
                    // Emergency pinned first, then grant-backed shares by recency.
                    val grantRows = grants
                        .filter { it.granteeKind != "emergency_authority" }
                        .map { ShareRow.fromGrant(it) }
                        .sortedByDescending { it.createdAtMs }
                    shares = listOf(ShareRow.emergency(emergencyCfg)) + grantRows
                    error = null
                }
                .onFailure { error = "Couldn't load shares: ${it.message}" }
        }
    }

    Scaffold(
        floatingActionButton = {
            ExtendedFloatingActionButton(
                onClick = { showCreate = true },
                icon = { Icon(Icons.Filled.Add, contentDescription = "New share") },
                text = { Text("New share") },
                containerColor = MaterialTheme.colorScheme.primary,
                contentColor = MaterialTheme.colorScheme.onPrimary,
            )
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
                Text(text = "Shares", style = MaterialTheme.typography.headlineSmall)
                Text(
                    text = "Each share is one party that may see a slice of your " +
                        "data. Toggle a share off to pause it instantly — its " +
                        "scope is kept and resumes on toggle.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(12.dp))

                if (error != null) {
                    Text(text = error!!, color = MaterialTheme.colorScheme.error)
                } else {
                    LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        items(shares, key = { it.id }) { share ->
                            ShareCard(
                                share = share,
                                onOpen = { onOpenShare(share.id) },
                                onToggle = { wantEnabled ->
                                    scope.launch(Dispatchers.IO) {
                                        applyToggle(ctx, share, wantEnabled)
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

    if (showCreate) {
        ModalBottomSheet(
            onDismissRequest = { showCreate = false },
            sheetState = sheetState,
        ) {
            CreateShareSheet(
                onSubmit = { tplId, label, purpose ->
                    val input = GrantTemplates.forTemplate(tplId, label, purpose)
                    // createGrant is a blocking network RPC — run it off the
                    // main thread, then apply UI state on the main dispatcher.
                    scope.launch(Dispatchers.IO) {
                        val result = StorageRepository.createGrant(input)
                        withContext(Dispatchers.Main) {
                            result.fold(
                                onSuccess = { res ->
                                    // Persist the bearer alongside the grant so
                                    // ShareDetailScreen can rebuild a working
                                    // share link without having to re-issue.
                                    GrantTokenStore.save(ctx, res.grantUlid, res.token)
                                    lastToken = res.token
                                    refreshTick++
                                },
                                onFailure = { e -> error = "Create share failed: ${e.message}" },
                            )
                            sheetState.hide()
                            showCreate = false
                        }
                    }
                },
                onCancel = {
                    scope.launch { sheetState.hide() }
                        .invokeOnCompletion { showCreate = false }
                },
            )
        }
    }

    lastToken?.let { token ->
        ModalBottomSheet(
            onDismissRequest = { lastToken = null },
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        ) {
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 24.dp, vertical = 16.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                Text("Share created", style = MaterialTheme.typography.titleLarge)
                Text(
                    "Open the new share to view its link and QR code. The grant " +
                        "token is shown again on the share detail screen.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                OutlinedTextField(
                    value = token,
                    onValueChange = {},
                    readOnly = true,
                    label = { Text("Grant token") },
                    modifier = Modifier.fillMaxWidth(),
                )
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.End,
                ) {
                    Button(onClick = { lastToken = null }) { Text("Done") }
                }
                Spacer(Modifier.height(16.dp))
            }
        }
    }
}

/**
 * Apply the row toggle. For grant-backed shares this flips
 * `suspended_at_ms`; for the synthetic emergency row it flips the emergency
 * feature master switch (the two are kept in lock-step).
 */
private suspend fun applyToggle(
    ctx: android.content.Context,
    share: ShareRow,
    wantEnabled: Boolean,
) {
    if (share.kind == ShareKind.Emergency) {
        val cfg = StorageRepository.getEmergencyConfig()
            .getOrDefault(EmergencyConfig())
        StorageRepository.setEmergencyConfig(cfg.copy(featureEnabled = wantEnabled))
    } else {
        StorageRepository.setGrantSuspended(share.id, suspended = !wantEnabled)
    }
}

@Composable
private fun ShareCard(
    share: ShareRow,
    onOpen: () -> Unit,
    onToggle: (Boolean) -> Unit,
) {
    val nowMs = System.currentTimeMillis()
    val expired = share.expired(nowMs)

    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
        modifier = Modifier.fillMaxWidth(),
        onClick = onOpen,
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(horizontal = 16.dp, vertical = 12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Column(modifier = Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        text = share.label,
                        style = MaterialTheme.typography.titleMedium,
                    )
                    Spacer(Modifier.width(8.dp))
                    ShareKindChip(share.kind)
                }
                Spacer(Modifier.height(4.dp))
                Text(
                    text = statusLine(share, expired, nowMs),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                if (share.pinned) {
                    Text(
                        text = "Pinned · break-glass · cannot be deleted",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
            // Quick enable/disable toggle. Revoked shares can't be toggled
            // back on — revocation is terminal; suspension is reversible.
            Switch(
                checked = share.enabled,
                onCheckedChange = onToggle,
                enabled = !share.revoked,
            )
        }
    }
}

@Composable
private fun ShareKindChip(kind: ShareKind) {
    val container = when (kind) {
        ShareKind.Emergency -> MaterialTheme.colorScheme.errorContainer
        ShareKind.Researcher -> MaterialTheme.colorScheme.tertiaryContainer
        ShareKind.Agent -> MaterialTheme.colorScheme.secondaryContainer
        else -> MaterialTheme.colorScheme.primaryContainer
    }
    AssistChip(
        onClick = {},
        label = { Text(kind.label, style = MaterialTheme.typography.labelSmall) },
        colors = AssistChipDefaults.assistChipColors(containerColor = container),
    )
}

private fun statusLine(share: ShareRow, expired: Boolean, nowMs: Long): String {
    val state = when {
        share.revoked -> "Revoked"
        !share.enabled -> "Paused"
        expired -> "Expired"
        else -> "Active"
    }
    val parts = mutableListOf(state)
    if (share.kind == ShareKind.Emergency) {
        parts.add("first responders, on approval")
    } else {
        share.grant?.let { g ->
            val scope = g.readEventTypes.size
            parts.add(if (scope == 0) "default scope" else "$scope event types")
            parts.add("expires " + (g.expiresAtMs?.let { fmtDate(it) } ?: "never"))
            if (g.useCount > 0) parts.add("${g.useCount} accesses")
        }
    }
    return parts.joinToString(" · ")
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun CreateShareSheet(
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
        Text("New share", style = MaterialTheme.typography.titleLarge)
        Text(
            "Pick a template — we set a sensible scope. Fine-tune it later from " +
                "the share's detail screen.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            // The emergency template is excluded — emergency is the pinned,
            // pre-configured share, not something you create from here.
            GrantTemplates.Id.entries
                .filter { it != GrantTemplates.Id.EMERGENCY_BREAK_GLASS }
                .forEach { id ->
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
            label = { Text("Grantee name (e.g. Dr Eva Novák)") },
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
            ) { Text("Create") }
        }
        Spacer(Modifier.height(16.dp))
    }
}

@Preview(showBackground = true, heightDp = 760)
@Composable
private fun SharesScreenPreview() {
    OhdConnectTheme {
        Surface { SharesScreen(contentPadding = PaddingValues(0.dp), onOpenShare = {}) }
    }
}
