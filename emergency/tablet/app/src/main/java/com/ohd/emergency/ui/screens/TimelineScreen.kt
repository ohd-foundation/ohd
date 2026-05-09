package com.ohd.emergency.ui.screens

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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.FilterChip
import androidx.compose.material3.FilterChipDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp

import com.ohd.emergency.data.CaseVault
import com.ohd.emergency.data.EmergencyRepository
import com.ohd.emergency.data.OperatorSession
import com.ohd.emergency.data.TimelineEntry
import com.ohd.emergency.data.TimelineKind
import com.ohd.emergency.ui.components.ChipTone
import com.ohd.emergency.ui.components.EmergencyTopBar
import com.ohd.emergency.ui.components.StatusChip
import com.ohd.emergency.ui.components.SyncIndicatorState
import com.ohd.emergency.ui.theme.MonoStyle
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * Case timeline.
 *
 * Chronological feed of every event recorded under the case grant — events
 * the case opened with (cloned from the patient's emergency template) plus
 * everything the crew has logged during the case. Most-recent-first.
 *
 * Filter chips: All / Vitals / Drugs / Observations / Notes. The "All"
 * chip is selected by default; tapping a category narrows the feed.
 *
 * Queued-but-not-flushed entries (offline writes that haven't reached
 * OHDC yet) get a small "Queued" badge so the responder knows the
 * timeline isn't synced authoritatively.
 *
 * Real impl: `OhdcService.QueryEvents(case_id = caseUlid).toList()`
 * unioned with `CaseVault.queuedWrites.value.filter { case match }`.
 * v0 calls `EmergencyRepository.loadTimeline(caseUlid)` which already
 * does the union over mock baseline + queued writes.
 */
@Composable
fun TimelineScreen(
    caseUlid: String,
    onOpenPatient: () -> Unit,
    onOpenIntervention: () -> Unit,
    onOpenHandoff: () -> Unit,
    onPanicLogout: () -> Unit,
    contentPadding: PaddingValues = PaddingValues(0.dp),
) {
    val ctx = LocalContext.current
    var entries by remember { mutableStateOf<List<TimelineEntry>>(emptyList()) }
    var filter by remember { mutableStateOf<TimelineFilter>(TimelineFilter.All) }
    val activeCase by CaseVault.activeCase.collectAsState()
    val syncStatus by CaseVault.syncStatus.collectAsState()
    val queuedWrites by CaseVault.queuedWrites.collectAsState()

    LaunchedEffect(caseUlid, queuedWrites.size) {
        entries = EmergencyRepository.loadTimeline(caseUlid)
    }

    val filtered = entries.filter { filter.accepts(it.kind) }

    Surface(modifier = Modifier.fillMaxSize()) {
        Column(modifier = Modifier.fillMaxSize().padding(contentPadding)) {
            EmergencyTopBar(
                operatorLabel = OperatorSession.operatorLabel(ctx),
                responderLabel = OperatorSession.responderLabel(ctx),
                syncStatus = SyncIndicatorState(syncStatus, queuedWrites.size),
                activeCaseShortLabel = activeCase?.caseUlid?.takeLast(6),
                onPanicLogout = onPanicLogout,
            )

            Column(
                modifier = Modifier.fillMaxSize().padding(horizontal = 24.dp, vertical = 16.dp),
            ) {
                Text(
                    text = "Case timeline",
                    style = MaterialTheme.typography.headlineMedium,
                    fontWeight = FontWeight.SemiBold,
                )
                Text(
                    text = "Case ${caseUlid.takeLast(8)} · ${entries.size} entries",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(12.dp))

                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    TimelineFilter.values().forEach { f ->
                        FilterChip(
                            selected = f == filter,
                            onClick = { filter = f },
                            label = { Text(f.label) },
                            colors = FilterChipDefaults.filterChipColors(
                                selectedContainerColor = MaterialTheme.colorScheme.primary,
                                selectedLabelColor = MaterialTheme.colorScheme.onPrimary,
                            ),
                        )
                    }
                }

                Spacer(Modifier.height(16.dp))

                if (filtered.isEmpty()) {
                    Box(modifier = Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        Text(
                            text = "No entries yet for this filter.",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                } else {
                    LazyColumn(
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                        modifier = Modifier.fillMaxSize().padding(bottom = 80.dp),
                    ) {
                        items(filtered) { e -> TimelineRow(e) }
                    }
                }
            }

            CaseNavBar(
                selected = CaseTab.Timeline,
                onPatient = onOpenPatient,
                onIntervention = onOpenIntervention,
                onTimeline = {},
                onHandoff = onOpenHandoff,
            )
        }
    }
}

@Composable
private fun TimelineRow(e: TimelineEntry) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.surfaceVariant,
        ),
    ) {
        Row(
            modifier = Modifier.fillMaxWidth().padding(16.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                text = formatTime(e.timestampMs),
                style = MonoStyle,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.fillMaxWidth(0.18f),
            )
            Column(modifier = Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    StatusChip(label = e.kind.name, tone = e.kind.tone(), outlined = true)
                    Spacer(modifier = Modifier.width(8.dp))
                    if (e.queuedNotFlushed) {
                        StatusChip(label = "Queued", tone = ChipTone.Warning)
                    }
                }
                Spacer(Modifier.height(6.dp))
                Text(
                    text = e.summary,
                    style = MaterialTheme.typography.bodyLarge,
                )
            }
        }
    }
}

private enum class TimelineFilter(val label: String, val kinds: Set<TimelineKind>) {
    All("All", TimelineKind.values().toSet()),
    Vitals("Vitals", setOf(TimelineKind.Vital)),
    Drugs("Drugs", setOf(TimelineKind.Drug)),
    Observations("Observations", setOf(TimelineKind.Observation, TimelineKind.Note)),
    System("System", setOf(TimelineKind.GrantOpened, TimelineKind.Handoff)),
    ;

    fun accepts(kind: TimelineKind): Boolean = kind in kinds
}

private fun TimelineKind.tone(): ChipTone = when (this) {
    TimelineKind.Vital -> ChipTone.Info
    TimelineKind.Drug -> ChipTone.Critical
    TimelineKind.Observation -> ChipTone.Neutral
    TimelineKind.Note -> ChipTone.Neutral
    TimelineKind.Handoff -> ChipTone.Warning
    TimelineKind.GrantOpened -> ChipTone.AutoGrant
}

private val timeFmt = SimpleDateFormat("HH:mm:ss", Locale.getDefault())
private fun formatTime(ms: Long): String = timeFmt.format(Date(ms))
