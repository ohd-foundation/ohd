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
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
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
import com.ohd.connect.data.CaseDetail
import com.ohd.connect.data.CaseSummary
import com.ohd.connect.data.CreateGrantInput
import com.ohd.connect.data.GrantTemplates
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.theme.OhdConnectTheme
import kotlinx.coroutines.launch

/**
 * Cases screen — surfaces active and closed cases per `connect/SPEC.md`
 * "Cases" + `connect/spec/screens-emergency.md` "Cases tab".
 *
 * Backs `Cases.{ListCases, GetCase, ForceCloseCase, IssueRetrospectiveGrant}`.
 * The repository's uniffi calls are TODO-stubs (see `StorageRepository.kt`);
 * UI is real.
 *
 * Layout:
 *  - Open/active cases prominent at top with elapsed time + active authority.
 *  - Tap a case → expanded detail (timeline placeholder, audit, handoff
 *    chain, force-close button).
 *  - Closed cases below, with "Issue retrospective grant" affordance.
 *  - "Auto-granted via timeout" badge in distinct colour per the
 *    designer-handoff doc.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CasesScreen(contentPadding: PaddingValues) {
    var cases by remember { mutableStateOf<List<CaseSummary>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }
    var refreshTick by remember { mutableStateOf(0) }
    var retroFor by remember { mutableStateOf<CaseSummary?>(null) }
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    val scope = rememberCoroutineScope()

    LaunchedEffect(refreshTick) {
        StorageRepository.listCases(includeClosed = true)
            .onSuccess {
                cases = it
                error = null
            }
            .onFailure { error = "Couldn't load cases: ${it.message}" }
    }

    val active = cases.filter { it.endedAtMs == null }
    val closed = cases.filter { it.endedAtMs != null }

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(contentPadding)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            Text(text = "Cases", style = MaterialTheme.typography.headlineSmall)
            Text(
                text = "Reads and writes that belong together — emergencies, hospital admissions, clinic visits.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(12.dp))

            when {
                error != null -> Text(text = error!!, color = MaterialTheme.colorScheme.error)
                cases.isEmpty() -> EmptyState()
                else -> {
                    LazyColumn(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        if (active.isNotEmpty()) {
                            item {
                                SectionHeader(label = "Active (${active.size})")
                            }
                            items(active, key = { it.ulid }) { c ->
                                CaseCard(
                                    case = c,
                                    onForceClose = { reason ->
                                        scope.launch {
                                            StorageRepository.forceCloseCase(c.ulid, reason)
                                            refreshTick++
                                        }
                                    },
                                    onIssueRetro = null,
                                )
                            }
                        }
                        if (closed.isNotEmpty()) {
                            item {
                                Spacer(Modifier.height(8.dp))
                                SectionHeader(label = "Closed (${closed.size})")
                            }
                            items(closed, key = { it.ulid }) { c ->
                                CaseCard(
                                    case = c,
                                    onForceClose = null,
                                    onIssueRetro = { retroFor = c },
                                )
                            }
                        }
                    }
                }
            }
        }
    }

    retroFor?.let { target ->
        ModalBottomSheet(
            onDismissRequest = { retroFor = null },
            sheetState = sheetState,
        ) {
            RetrospectiveGrantSheet(
                case = target,
                onSubmit = { tplId, label, purpose ->
                    val input = GrantTemplates.forTemplate(tplId, label, purpose)
                    scope.launch {
                        StorageRepository.issueRetrospectiveGrant(target.ulid, input)
                        sheetState.hide()
                        retroFor = null
                        refreshTick++
                    }
                },
                onCancel = {
                    scope.launch { sheetState.hide() }
                        .invokeOnCompletion { retroFor = null }
                },
            )
        }
    }
}

@Composable
private fun EmptyState() {
    Box(
        modifier = Modifier.fillMaxSize().padding(top = 64.dp),
        contentAlignment = Alignment.TopCenter,
    ) {
        Text(
            text = "No active or recent cases.\nWhen emergency responders or providers access your OHD via cases, you'll see them here.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun SectionHeader(label: String) {
    Text(
        text = label,
        style = MaterialTheme.typography.labelLarge,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = Modifier.padding(vertical = 4.dp),
    )
}

@Composable
private fun CaseCard(
    case: CaseSummary,
    onForceClose: ((reason: String?) -> Unit)?,
    onIssueRetro: (() -> Unit)?,
) {
    var expanded by remember { mutableStateOf(false) }
    var detail by remember { mutableStateOf<CaseDetail?>(null) }
    var loadError by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(expanded, case.ulid) {
        if (expanded && detail == null) {
            StorageRepository.getCase(case.ulid)
                .onSuccess { detail = it; loadError = null }
                .onFailure { loadError = "Couldn't load: ${it.message}" }
        }
    }

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
                    Text(
                        text = case.label ?: prettyCaseType(case.caseType),
                        style = MaterialTheme.typography.titleMedium,
                    )
                    Text(
                        text = case.authorityLabel ?: prettyCaseType(case.caseType),
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                CaseStatusFlag(case = case)
            }
            Spacer(Modifier.height(6.dp))

            if (case.endedAtMs == null) {
                Text(
                    text = "Started ${fmtRelative(case.startedAtMs)} · running ${fmtElapsed(case.startedAtMs)}",
                    style = MaterialTheme.typography.bodySmall,
                )
            } else {
                Text(
                    text = "Ran ${fmtElapsed(case.startedAtMs, case.endedAtMs)} · ended ${fmtRelative(case.endedAtMs)}",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }

            if (case.autoGranted) {
                Spacer(Modifier.height(6.dp))
                AutoGrantedBadge()
            }

            AnimatedVisibility(visible = expanded) {
                Column(modifier = Modifier.padding(top = 8.dp)) {
                    HorizontalDivider()
                    Spacer(Modifier.height(8.dp))

                    if (loadError != null) {
                        Text(loadError!!, color = MaterialTheme.colorScheme.error)
                    }
                    detail?.let { d ->
                        Text(
                            text = "Audit (${d.audit.size} entries)",
                            style = MaterialTheme.typography.labelMedium,
                        )
                        Spacer(Modifier.height(4.dp))
                        d.audit.take(5).forEach { entry ->
                            Text(
                                text = "${fmtRelative(entry.tsMs)} · ${entry.opName} · ${entry.actorLabel}" +
                                        (entry.rowsReturned?.let { " · ${it} rows" } ?: "") +
                                        if (entry.autoGranted) " · auto" else "",
                                style = MaterialTheme.typography.bodySmall,
                            )
                        }
                        if (d.handoffChain.isNotEmpty()) {
                            Spacer(Modifier.height(8.dp))
                            Text(
                                text = "Handoff chain",
                                style = MaterialTheme.typography.labelMedium,
                            )
                            d.handoffChain.forEach { h ->
                                Text(
                                    text = "${fmtDate(h.tsMs)} · ${h.authorityLabel}" +
                                            (h.toAuthority?.let { " → $it" } ?: ""),
                                    style = MaterialTheme.typography.bodySmall,
                                )
                            }
                        }
                    }

                    Spacer(Modifier.height(12.dp))
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        if (onForceClose != null) {
                            Button(
                                onClick = { onForceClose(null) },
                                colors = ButtonDefaults.buttonColors(
                                    containerColor = MaterialTheme.colorScheme.error,
                                    contentColor = MaterialTheme.colorScheme.onError,
                                ),
                            ) { Text("Force close") }
                        }
                        if (onIssueRetro != null) {
                            OutlinedButton(onClick = onIssueRetro) {
                                Text("Issue retrospective grant")
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun CaseStatusFlag(case: CaseSummary) {
    val (label, container) = when {
        case.endedAtMs != null -> "closed" to MaterialTheme.colorScheme.surfaceVariant
        case.caseType == "emergency" -> "active · emergency" to MaterialTheme.colorScheme.errorContainer
        else -> "active" to MaterialTheme.colorScheme.primaryContainer
    }
    AssistChip(
        onClick = {},
        label = { Text(label, style = MaterialTheme.typography.labelSmall) },
        colors = AssistChipDefaults.assistChipColors(containerColor = container),
    )
}

@Composable
private fun AutoGrantedBadge() {
    AssistChip(
        onClick = {},
        label = {
            Text(
                "Auto-granted via timeout",
                style = MaterialTheme.typography.labelSmall,
            )
        },
        colors = AssistChipDefaults.assistChipColors(
            containerColor = MaterialTheme.colorScheme.tertiaryContainer,
            labelColor = MaterialTheme.colorScheme.onTertiaryContainer,
        ),
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RetrospectiveGrantSheet(
    case: CaseSummary,
    onSubmit: (tplId: GrantTemplates.Id, label: String, purpose: String?) -> Unit,
    onCancel: () -> Unit,
) {
    var template by remember { mutableStateOf(GrantTemplates.Id.SPECIALIST_VISIT) }
    var label by remember { mutableStateOf("") }
    var purpose by remember { mutableStateOf("") }

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 24.dp, vertical = 16.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        Text("Issue retrospective grant", style = MaterialTheme.typography.titleLarge)
        Text(
            "Scoped to events recorded during ${case.label ?: "this case"} (${fmtDate(case.startedAtMs)}" +
                    "${case.endedAtMs?.let { " → " + fmtDate(it) } ?: ""}).",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )

        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            // Limit to specialist / researcher templates — these are the
            // typical retrospective-grant cases (specialist consult,
            // insurer billing review). The user can still edit on the
            // resulting grant detail screen.
            listOf(
                GrantTemplates.Id.SPECIALIST_VISIT,
                GrantTemplates.Id.RESEARCHER,
                GrantTemplates.Id.PRIMARY_DOCTOR,
            ).forEach { id ->
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
            label = { Text("Grantee label (e.g. Insurer billing review)") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth(),
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

private fun prettyCaseType(t: String): String = when (t) {
    "emergency" -> "Emergency"
    "hospital_admission" -> "Hospital admission"
    "clinic_visit" -> "Clinic visit"
    "specialist_consult" -> "Specialist consult"
    else -> t.replace('_', ' ').replaceFirstChar { it.uppercaseChar() }
}

@Preview(showBackground = true, heightDp = 720)
@Composable
private fun CasesScreenPreview() {
    OhdConnectTheme {
        Surface { CasesScreen(contentPadding = PaddingValues(0.dp)) }
    }
}
