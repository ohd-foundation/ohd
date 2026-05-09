package com.ohd.connect.ui.screens

import androidx.compose.foundation.ExperimentalFoundationApi
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
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.lazy.stickyHeader
import androidx.compose.material3.AssistChip
import androidx.compose.material3.AssistChipDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.tooling.preview.Preview
import androidx.compose.ui.unit.dp
import com.ohd.connect.data.AuditEntry
import com.ohd.connect.data.AuditFilter
import com.ohd.connect.data.StorageRepository
import com.ohd.connect.ui.theme.OhdConnectTheme
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Audit screen — surfaces `Audit.AuditQuery` rows.
 *
 * Filters:
 *  - Op kind (read / write / grant_mgmt). Multi-select chips.
 *  - Time range (24h / 7d / 30d / all). Single-select chips.
 *
 * Each row: actor (self / grant), op name, query summary, rows_returned,
 * rows_filtered, plus an `auto_granted` indicator for break-glass timeout
 * entries (per `connect/spec/screens-emergency.md` "designer handoff").
 *
 * Grouped by day with a sticky day header. Repository's uniffi calls are
 * TODO-stubs (see `StorageRepository.kt`). UI is real.
 */
@OptIn(ExperimentalFoundationApi::class)
@Composable
fun AuditScreen(contentPadding: PaddingValues) {
    var opReads by remember { mutableStateOf(true) }
    var opWrites by remember { mutableStateOf(true) }
    var opMgmt by remember { mutableStateOf(true) }
    var range by remember { mutableStateOf(TimeRange.LAST_7D) }

    var rows by remember { mutableStateOf<List<AuditEntry>>(emptyList()) }
    var error by remember { mutableStateOf<String?>(null) }

    LaunchedEffect(opReads, opWrites, opMgmt, range) {
        val opKinds = buildList {
            if (opReads) add("read")
            if (opWrites) add("write")
            if (opMgmt) add("grant_mgmt")
        }
        val now = System.currentTimeMillis()
        val from = when (range) {
            TimeRange.LAST_24H -> now - 86_400_000L
            TimeRange.LAST_7D -> now - 7L * 86_400_000L
            TimeRange.LAST_30D -> now - 30L * 86_400_000L
            TimeRange.ALL -> null
        }
        StorageRepository.auditQuery(
            AuditFilter(
                opKindsIn = opKinds,
                fromMs = from,
                limit = 500,
            ),
        )
            .onSuccess { rows = it; error = null }
            .onFailure { error = "Couldn't load audit: ${it.message}" }
    }

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(
            modifier = Modifier
                .fillMaxSize()
                .padding(contentPadding)
                .padding(horizontal = 16.dp, vertical = 12.dp),
        ) {
            Text(text = "Audit", style = MaterialTheme.typography.headlineSmall)
            Text(
                text = "Every read and write under your data. Auto-granted entries are emergency-timeout fallbacks.",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Spacer(Modifier.height(12.dp))

            // Op-kind chips
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                FilterChip(
                    selected = opReads,
                    onClick = { opReads = !opReads },
                    label = { Text("read") },
                )
                FilterChip(
                    selected = opWrites,
                    onClick = { opWrites = !opWrites },
                    label = { Text("write") },
                )
                FilterChip(
                    selected = opMgmt,
                    onClick = { opMgmt = !opMgmt },
                    label = { Text("grant mgmt") },
                )
            }
            Spacer(Modifier.height(6.dp))

            // Time-range chips
            Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                TimeRange.entries.forEach { r ->
                    FilterChip(
                        selected = range == r,
                        onClick = { range = r },
                        label = { Text(r.label) },
                        colors = FilterChipDefaults.filterChipColors(),
                    )
                }
            }
            Spacer(Modifier.height(12.dp))

            val errorMsg = error
            when {
                errorMsg != null -> Text(text = errorMsg, color = MaterialTheme.colorScheme.error)
                rows.isEmpty() -> EmptyAudit()
                else -> AuditList(rows = rows)
            }
        }
    }
}

private enum class TimeRange(val label: String) {
    LAST_24H("24h"),
    LAST_7D("7d"),
    LAST_30D("30d"),
    ALL("all"),
}

@Composable
private fun EmptyAudit() {
    Box(
        modifier = Modifier.fillMaxSize().padding(top = 64.dp),
        contentAlignment = Alignment.TopCenter,
    ) {
        Text(
            text = "No audit entries match these filters.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun AuditList(rows: List<AuditEntry>) {
    val grouped = remember(rows) {
        rows
            .sortedByDescending { it.tsMs }
            .groupBy { dayKey(it.tsMs) }
            .toList() // List<Pair<dayKey, List<AuditEntry>>>
    }
    val state = rememberLazyListState()

    LazyColumn(
        state = state,
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        grouped.forEach { (day, items) ->
            stickyHeader {
                DayHeader(day = day, count = items.size)
            }
            items(items, key = { it.ulid }) { row -> AuditRow(row) }
        }
    }
}

@Composable
private fun DayHeader(day: String, count: Int) {
    Surface(
        color = MaterialTheme.colorScheme.surface,
        modifier = Modifier.fillMaxWidth(),
    ) {
        Row(
            modifier = Modifier.padding(vertical = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = day,
                style = MaterialTheme.typography.titleSmall,
            )
            Text(
                text = "$count",
                style = MaterialTheme.typography.labelSmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun AuditRow(row: AuditEntry) {
    Card(
        colors = CardDefaults.cardColors(
            containerColor = if (row.autoGranted) {
                MaterialTheme.colorScheme.tertiaryContainer
            } else {
                MaterialTheme.colorScheme.surfaceVariant
            },
        ),
        modifier = Modifier.fillMaxWidth(),
    ) {
        Column(modifier = Modifier.padding(horizontal = 14.dp, vertical = 10.dp)) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Column(modifier = Modifier.weight(1f)) {
                    Text(
                        text = row.opName,
                        style = MaterialTheme.typography.titleSmall,
                    )
                    Text(
                        text = "${row.actorLabel} · ${row.actorType}",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                OpKindChip(opKind = row.opKind, autoGranted = row.autoGranted)
            }
            if (!row.querySummary.isNullOrBlank()) {
                Spacer(Modifier.height(4.dp))
                Text(
                    text = row.querySummary,
                    style = MaterialTheme.typography.bodySmall.copy(fontFamily = FontFamily.Monospace),
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
            Spacer(Modifier.height(2.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                Text(
                    text = "${TimeFmt.format(Date(row.tsMs))}",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                row.rowsReturned?.let {
                    Text(
                        text = "$it rows",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                row.rowsFiltered?.takeIf { it > 0 }?.let {
                    Text(
                        text = "$it filtered",
                        style = MaterialTheme.typography.labelSmall,
                        color = MaterialTheme.colorScheme.error,
                    )
                }
            }
        }
    }
}

@Composable
private fun OpKindChip(opKind: String, autoGranted: Boolean) {
    val (label, container) = when {
        autoGranted -> "auto" to MaterialTheme.colorScheme.tertiary
        opKind == "read" -> "read" to MaterialTheme.colorScheme.secondaryContainer
        opKind == "write" -> "write" to MaterialTheme.colorScheme.primaryContainer
        opKind == "grant_mgmt" -> "grant" to MaterialTheme.colorScheme.surface
        else -> opKind to MaterialTheme.colorScheme.surfaceVariant
    }
    AssistChip(
        onClick = {},
        label = { Text(label, style = MaterialTheme.typography.labelSmall) },
        colors = AssistChipDefaults.assistChipColors(containerColor = container),
    )
}

private val DayFmt = SimpleDateFormat("yyyy-MM-dd", Locale.getDefault())
private val TimeFmt = SimpleDateFormat("HH:mm:ss", Locale.getDefault())

private fun dayKey(ms: Long): String = DayFmt.format(Date(ms))

@Preview(showBackground = true, heightDp = 720)
@Composable
private fun AuditScreenPreview() {
    OhdConnectTheme {
        Surface { AuditScreen(contentPadding = PaddingValues(0.dp)) }
    }
}
